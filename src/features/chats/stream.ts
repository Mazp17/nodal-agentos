// Live state of each chat: its history (read once from the Claude Code session) plus what its
// dedicated channel streams after that (P05: one `Channel` per chat, attached before its first
// message and kept for the app's session — not a listener shared by every chat).

import { Channel } from "@tauri-apps/api/core";
import { useCallback, useEffect, useSyncExternalStore } from "react";
import {
  attachChatChannel,
  detachChatChannel,
  listChats,
  getChatLive,
  getChatTranscript,
  sendChatMessage,
  type ChatEvent,
  type ChatEventEnvelope,
  type ChatRunState,
  type SubagentEvent,
  type PermissionRequest,
} from "../../domain/api";
import { readJsonPref, writePref } from "../../shell/storage";
import type { TranscriptImage, TranscriptItem } from "./types";
import { appFocused, notifyChat, preview } from "./alerts";
import { chatTitle, toolLabel, type Outgoing } from "./model";

export interface ChatStream {
  /** `false` while the history loads. */
  loaded: boolean;
  loadError: string | null;
  /** Older items the history left out. */
  omitted: number;
  /** History, then every complete item streamed since. */
  items: TranscriptItem[];
  /** Messages sent that Claude hasn't echoed back yet. */
  outbox: Outgoing[];
  /** Text of the answer being written (a later `item` carries it in full). */
  streaming: string;
  thinking: boolean;
  /** Text of the thinking being written (a later `item` carries it in full). */
  thinkingText: string;
  state: ChatRunState;
  /** Permission requests waiting for an answer, oldest first. */
  pending: PermissionRequest[];
  /** Tool calls denied without asking, by tool use id. */
  denied: Record<string, string>;
  /** What each subagent did since the chat loaded, by the id of the `Agent` call that launched it. */
  subagents: Record<string, TranscriptItem[]>;
  contextTokens: number | null;
  contextWindow: number | null;
  /** Why the last turn failed, until the next message. */
  notice: string | null;
  interrupting: boolean;
  /** Shells running since the chat loaded, by the id of the `Bash` call that started them. */
  shells: Record<string, Shell>;
}

/** A `Bash` call still running: a foreground one until its result, a background one until it ends. */
export interface Shell {
  toolUseId: string;
  /** Set once Claude Code reports it as a task (background shells): what stopping it takes. */
  taskId: string | null;
  command: string;
  description: string | null;
  background: boolean;
  startedAt: number;
}

const EMPTY: ChatStream = {
  loaded: false,
  loadError: null,
  omitted: 0,
  items: [],
  outbox: [],
  streaming: "",
  thinking: false,
  thinkingText: "",
  state: "stopped",
  pending: [],
  denied: {},
  subagents: {},
  contextTokens: null,
  contextWindow: null,
  notice: null,
  interrupting: false,
  shells: {},
};

const streams = new Map<string, ChatStream>();
/** Run state of every chat that reported one, for the sessions list. */
let runStates: ReadonlyMap<string, ChatRunState> = new Map();
const listeners = new Map<string, Set<() => void>>();
const stateListeners = new Set<() => void>();
const loading = new Map<string, Promise<void>>();

function notify(chatId: string) {
  listeners.get(chatId)?.forEach((l) => l());
}

function set(chatId: string, update: (s: ChatStream) => ChatStream) {
  const cur = streams.get(chatId);
  if (!cur) return;
  const next = update(cur);
  streams.set(chatId, next);
  setNeeds(chatId, needOf(next));
  notify(chatId);
}

// ---------- Needs you ----------

/** What a chat waits on from the user: an approval, or a turn that failed. */
export type ChatNeed =
  /** `question`: Claude asks via `AskUserQuestion`, answered only in the chat. */
  | { kind: "approval"; requestId: string; tool: string; summary: string | null; more: number; question: boolean }
  | { kind: "failed"; message: string };

let needs: ReadonlyMap<string, ChatNeed> = new Map();
const needsListeners = new Set<() => void>();

function needOf(s: ChatStream): ChatNeed | null {
  const p = s.pending[0];
  if (p) {
    const question = p.toolName === "AskUserQuestion";
    return { kind: "approval", requestId: p.requestId, tool: toolLabel(p.toolName), summary: p.summary, more: s.pending.length - 1, question };
  }
  if (s.notice && s.state !== "busy") return { kind: "failed", message: s.notice };
  return null;
}

function setNeeds(chatId: string, need: ChatNeed | null) {
  const cur = needs.get(chatId) ?? null;
  if (cur === need || (cur && need && JSON.stringify(cur) === JSON.stringify(need))) return;
  const next = new Map(needs);
  if (need) next.set(chatId, need);
  else next.delete(chatId);
  needs = next;
  needsListeners.forEach((l) => l());
}

/** First line of the chat's last answer, if its history is loaded this session. */
export function lastAnswer(chatId: string): string | null {
  const items = streams.get(chatId)?.items ?? [];
  const last = [...items].reverse().find((i) => i.kind === "text");
  return last?.kind === "text" ? preview(last.text, 140) || null : null;
}

/** Chats waiting on the user, for the overview's "Needs you" (only chats loaded this session). */
export function useChatNeeds(): ReadonlyMap<string, ChatNeed> {
  const subscribe = useCallback((l: () => void) => {
    needsListeners.add(l);
    return () => needsListeners.delete(l);
  }, []);
  return useSyncExternalStore(subscribe, () => needs);
}

// ---------- Finished at ----------

/** When each chat last stopped working (busy → anything else), kept across launches: the overview's "just finished". */
const FINISHED_PREF = "chatFinishedAt";
const isTimes = (v: unknown): v is Record<string, number> =>
  !!v && typeof v === "object" && !Array.isArray(v) && Object.values(v).every((x) => typeof x === "number");
let finishedAt: ReadonlyMap<string, number> = new Map(Object.entries(readJsonPref(FINISHED_PREF, {}, isTimes)));
const finishedListeners = new Set<() => void>();
const FINISHED_KEEP = 50;

function markFinished(chatId: string) {
  // Newest first, capped: only the latest few are ever shown.
  const rest = [...finishedAt].filter(([id]) => id !== chatId);
  const next = new Map<string, number>([[chatId, Date.now()] as const, ...rest].slice(0, FINISHED_KEEP));
  finishedAt = next;
  writePref(FINISHED_PREF, JSON.stringify(Object.fromEntries(next)));
  finishedListeners.forEach((l) => l());
}

export function useFinishedAt(): ReadonlyMap<string, number> {
  const subscribe = useCallback((l: () => void) => {
    finishedListeners.add(l);
    return () => finishedListeners.delete(l);
  }, []);
  return useSyncExternalStore(subscribe, () => finishedAt);
}

function setRunState(chatId: string, state: ChatRunState | null) {
  if ((runStates.get(chatId) ?? null) === state) return;
  if (runStates.get(chatId) === "busy") markFinished(chatId);
  const next = new Map(runStates);
  if (state) next.set(chatId, state);
  else next.delete(chatId);
  runStates = next;
  stateListeners.forEach((l) => l());
}

// ---------- Unread ----------

/** Chats with an answer or a question the user hasn't seen → their project, kept across launches. */
const UNREAD_PREF = "chatUnread";
const isUnread = (v: unknown): v is Record<string, string> =>
  !!v && typeof v === "object" && !Array.isArray(v) && Object.values(v).every((x) => typeof x === "string");
let unread: ReadonlyMap<string, string> = new Map(Object.entries(readJsonPref(UNREAD_PREF, {}, isUnread)));
const unreadListeners = new Set<() => void>();
/** The chat on the Chat page, or `null` on any other page. */
let viewed: string | null = null;

/** Marks `chatId` unread in `projectId`, or read with `null`. */
function setUnread(chatId: string, projectId: string | null) {
  if ((unread.get(chatId) ?? null) === projectId) return;
  const next = new Map(unread);
  if (projectId) next.set(chatId, projectId);
  else next.delete(chatId);
  unread = next;
  writePref(UNREAD_PREF, JSON.stringify(Object.fromEntries(next)));
  unreadListeners.forEach((l) => l());
}

/** Drops unread chats of projects that no longer exist (deleted with their chats). */
export function pruneUnread(projectIds: ReadonlySet<string>) {
  for (const [chatId, projectId] of unread) if (!projectIds.has(projectId)) setUnread(chatId, null);
}

/** The Chat page shows `chatId` (`null` when it closes): it's read. */
export function setViewedChat(chatId: string | null) {
  viewed = chatId;
  if (chatId) setUnread(chatId, null);
}

/** Something in `chatId` needs the user: unread if it isn't the chat open, notified unless on screen. */
async function flag(chatId: string, body: string) {
  const chat = await listChats(null).then(
    (cs) => cs.find((c) => c.id === chatId),
    () => undefined,
  );
  if (chat && viewed !== chatId) setUnread(chatId, chat.projectId);
  const focused = await appFocused();
  // On screen: Nodal in front with this chat open.
  if (!(focused && viewed === chatId)) void notifyChat(chatId, chat ? chatTitle(chat) : "Chat", body, focused);
}

/** Answer text for a notification: the turn's result, else the last text streamed. */
function answerOf(chatId: string, result: string | null): string {
  const items = streams.get(chatId)?.items ?? [];
  const last = [...items].reverse().find((i) => i.kind === "text");
  return preview(result) || preview(last?.kind === "text" ? last.text : null) || "Claude answered.";
}

/** The last turn's notice, held until the chat goes idle: messages queued mid-answer notify once. */
const held = new Map<string, string>();

function alertOn(chatId: string, e: ChatEvent) {
  if (e.type === "turnEnd") {
    // A stop the user asked for needs no notice.
    if (!e.ok && streams.get(chatId)?.interrupting) held.delete(chatId);
    else held.set(chatId, e.ok ? answerOf(chatId, e.result) : `The answer ended early${e.result ? `: ${preview(e.result)}` : "."}`);
  } else if (e.type === "error") {
    // The process died mid-answer: no `turnEnd` follows, only `state: stopped`.
    held.set(chatId, `The chat stopped: ${preview(e.message)}`);
  } else if (e.type === "state" && e.state !== "busy") {
    const body = held.get(chatId);
    held.delete(chatId);
    if (body) void flag(chatId, body);
  } else if (e.type === "permissionRequest") {
    if (streams.get(chatId)?.pending.some((p) => p.requestId === e.requestId)) return;
    void flag(chatId, `Needs your approval to use ${toolLabel(e.toolName)}${e.summary ? `: ${preview(e.summary, 100)}` : ""}`);
  }
}

const withResult = (items: TranscriptItem[], id: string, apply: (t: Extract<TranscriptItem, { kind: "toolUse" }>) => TranscriptItem) => {
  const i = items.findIndex((it) => it.kind === "toolUse" && it.id === id);
  if (i < 0) return items;
  const next = [...items];
  next[i] = apply(items[i] as Extract<TranscriptItem, { kind: "toolUse" }>);
  return next;
};

/** In-memory bounds of `subagents`: the most recent agents, the latest items of each. */
const SUBAGENTS_KEEP = 40;
const SUBAGENT_ITEMS_KEEP = 300;

const isKnownCall = (items: TranscriptItem[], it: TranscriptItem) =>
  it.kind === "toolUse" && !!it.id && items.some((x) => x.kind === "toolUse" && x.id === it.id);

/** Folds one subagent event into the items of its parent `Agent` call. */
export function reduceSubagent(
  subagents: Record<string, TranscriptItem[]>,
  parentToolUseId: string,
  e: SubagentEvent,
): Record<string, TranscriptItem[]> {
  const cur = subagents[parentToolUseId] ?? [];
  let items: TranscriptItem[];
  if (e.type === "toolResult") {
    items = withResult(cur, e.toolUseId, (t) => ({ ...t, result: e.result }));
  } else if (isKnownCall(cur, e.item)) {
    return subagents;
  } else {
    items = [...cur, e.item].slice(-SUBAGENT_ITEMS_KEEP);
  }
  // Re-inserting moves the key last, so the oldest agents are the ones dropped.
  const { [parentToolUseId]: _, ...rest } = subagents;
  return Object.fromEntries(Object.entries({ ...rest, [parentToolUseId]: items }).slice(-SUBAGENTS_KEEP));
}

/** Chats with something unread → their project. */
export function useUnreadChats(): ReadonlyMap<string, string> {
  const subscribe = useCallback((l: () => void) => {
    unreadListeners.add(l);
    return () => unreadListeners.delete(l);
  }, []);
  return useSyncExternalStore(subscribe, () => unread);
}

const INTERRUPTED = "[Request interrupted by user]";

/** A streamed `Bash` call as a running shell (its input is the call's pretty-printed JSON). */
function shellOf(it: Extract<TranscriptItem, { kind: "toolUse" }>): Shell | null {
  if (it.name !== "Bash" || !it.id || it.result) return null;
  let input: { command?: unknown; description?: unknown; run_in_background?: unknown } = {};
  try {
    input = JSON.parse(it.input ?? "{}");
  } catch {
    // Truncated input: fall back to the summary.
  }
  const command = typeof input.command === "string" ? input.command : (it.summary ?? "");
  const description = typeof input.description === "string" ? input.description : null;
  return { toolUseId: it.id, taskId: null, command, description, background: input.run_in_background === true, startedAt: Date.now() };
}

const without = (shells: Record<string, Shell>, keep: (sh: Shell) => boolean) => {
  const next = Object.fromEntries(Object.entries(shells).filter(([, sh]) => keep(sh)));
  return Object.keys(next).length === Object.keys(shells).length ? shells : next;
};

/** Applies one streamed event (exported for tests of the reducer). */
export function reduce(s: ChatStream, e: ChatEvent): ChatStream {
  switch (e.type) {
    case "init":
      return s;
    case "textDelta":
      return { ...s, streaming: s.streaming + e.text, thinking: false, thinkingText: "" };
    case "thinkingDelta":
      return { ...s, thinking: true, thinkingText: s.thinkingText + e.text };
    case "item": {
      const it = e.item;
      if (it.kind === "user") {
        // Echoes come in order, but Claude Code may merge queued messages into one echo (after
        // a stop) and adds user-side markers of its own: drop what this echo covers, by text.
        const i = s.outbox.map((m) => it.text.includes(m.text)).lastIndexOf(true);
        return { ...s, items: [...s.items, it], outbox: s.outbox.slice(i + 1) };
      }
      // Replayed after a reload of the history: keep the one already there.
      if (it.kind === "toolUse" && it.id && s.items.some((x) => x.kind === "toolUse" && x.id === it.id)) return s;
      const shell = it.kind === "toolUse" ? shellOf(it) : null;
      return {
        ...s,
        items: [...s.items, it],
        streaming: it.kind === "text" ? "" : s.streaming,
        thinking: false,
        thinkingText: "",
        shells: shell ? { ...s.shells, [shell.toolUseId]: shell } : s.shells,
      };
    }
    case "toolResult":
      return {
        ...s,
        items: withResult(s.items, e.toolUseId, (t) => ({ ...t, result: e.result })),
        // A background shell's result only says it started: it runs until `taskEnded`.
        shells: without(s.shells, (sh) => sh.toolUseId !== e.toolUseId || sh.background || sh.taskId != null),
      };
    case "shellStarted": {
      const sh = e.toolUseId ? s.shells[e.toolUseId] : undefined;
      if (!sh) return s;
      return { ...s, shells: { ...s.shells, [sh.toolUseId]: { ...sh, taskId: e.taskId, background: sh.background || e.background, description: sh.description ?? e.description } } };
    }
    case "taskEnded":
      return { ...s, shells: without(s.shells, (sh) => sh.taskId !== e.taskId) };
    case "subagent":
      return { ...s, subagents: reduceSubagent(s.subagents, e.parentToolUseId, e.event) };
    case "permissionRequest": {
      if (s.pending.some((p) => p.requestId === e.requestId)) return s;
      const { type: _, ...req } = e;
      return { ...s, pending: [...s.pending, req] };
    }
    case "permissionResolved":
      return { ...s, pending: s.pending.filter((p) => p.requestId !== e.requestId) };
    case "permissionDenied":
      return e.toolUseId ? { ...s, denied: { ...s.denied, [e.toolUseId]: e.message ?? "Denied" } } : s;
    case "usage":
      return { ...s, contextTokens: e.contextTokens };
    case "turnEnd": {
      // A stop the user asked for shows as the marker Claude Code writes to the session (the
      // live stream doesn't carry it), so it looks the same before and after a reload.
      const stopped = !e.ok && s.interrupting;
      const notice = e.ok || stopped ? null : (e.result ?? `The answer ended early (${e.subtype}).`);
      return {
        ...s,
        items: stopped ? [...s.items, { kind: "user", text: INTERRUPTED, truncated: false }] : s.items,
        streaming: "",
        thinking: false,
        thinkingText: "",
        interrupting: false,
        // A foreground call left without a result was cut off with the turn.
        shells: without(s.shells, (sh) => sh.background || sh.taskId != null),
        notice,
        contextWindow: e.contextWindow ?? s.contextWindow,
      };
    }
    case "state":
      return e.state === "busy"
        ? { ...s, state: e.state }
        : // Idle only comes once Claude has read every message sent, so nothing is left queued.
          { ...s, state: e.state, outbox: [], streaming: "", thinking: false, thinkingText: "", interrupting: false, ...(e.state === "stopped" ? { pending: [], shells: {} } : {}) };
    case "error":
      return { ...s, notice: e.message, outbox: [], streaming: "", thinking: false, thinkingText: "", interrupting: false };
  }
}

/** One attach (in flight or done) per chat id we've ever touched, so a repeat call is free. */
const attached = new Map<string, Promise<void>>();

function handle({ chatId, event }: ChatEventEnvelope) {
  if (event.type === "state") setRunState(chatId, event.state);
  // Before reducing: `turnEnd` clears `interrupting`.
  alertOn(chatId, event);
  if (streams.has(chatId)) set(chatId, (s) => reduce(s, event));
}

/**
 * Attaches `chatId`'s dedicated channel (P05): from then on its events arrive at `handle`.
 * Idempotent and cached, so callers can call it freely; `send()` awaits it once before a
 * chat's first message, so the process can't start (and stream) before something is listening.
 * No HMR dedup needed here (unlike a shared listener would): the backend keeps one channel per
 * chat id, so a second `attach` (an edited copy of this module reattaching) simply replaces the
 * first instead of both delivering.
 */
export function ensureChatAttached(chatId: string): Promise<void> {
  if (typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return Promise.resolve();
  const cached = attached.get(chatId);
  if (cached) return cached;
  const channel = new Channel<ChatEventEnvelope>();
  channel.onmessage = handle;
  const p = attachChatChannel(chatId, channel).catch((err: unknown) => {
    attached.delete(chatId);
    throw err;
  });
  attached.set(chatId, p);
  return p;
}

/** Reads the chat's history and live state. `fresh`: a new chat with nothing to read. */
function load(chatId: string, fresh = false): Promise<void> {
  const inflight = loading.get(chatId);
  if (inflight) return inflight;
  const prev = streams.get(chatId);
  streams.set(chatId, {
    ...EMPTY,
    contextTokens: prev?.contextTokens ?? null,
    contextWindow: prev?.contextWindow ?? null,
    loaded: fresh,
  });
  notify(chatId);
  if (fresh) return Promise.resolve();
  const job = Promise.all([getChatTranscript(chatId), getChatLive(chatId)])
    .then(([t, live]) => {
      setRunState(chatId, live.state);
      // Anything streamed while the history loaded goes after it.
      set(chatId, (s) => ({
        ...s,
        loaded: true,
        omitted: t?.omitted ?? 0,
        items: [...(t?.items ?? []), ...s.items],
        state: live.state,
        pending: live.pending,
      }));
    })
    .catch((e: unknown) => set(chatId, (s) => ({ ...s, loaded: true, loadError: String(e) })))
    .finally(() => loading.delete(chatId));
  loading.set(chatId, job);
  return job;
}

/** A chat just created here: nothing to read yet. */
export function seedChat(chatId: string) {
  void ensureChatAttached(chatId);
  if (!streams.has(chatId)) void load(chatId, true);
}

/** Before sending: shows the message until Claude echoes it. `queued`: sent while Claude was working. */
export function pushOutbox(chatId: string, text: string, queued = false, images: TranscriptImage[] = []) {
  // A stop that got no answer (the turn had just ended) must not mark the next one as stopped.
  set(chatId, (s) => ({ ...s, outbox: [...s.outbox, { text, queued, images }], notice: null, interrupting: queued && s.interrupting }));
}

export function dropOutbox(chatId: string, text: string) {
  set(chatId, (s) => {
    const i = s.outbox.map((m) => m.text).lastIndexOf(text);
    return i < 0 ? s : { ...s, outbox: s.outbox.filter((_, j) => j !== i) };
  });
}

/** After a rewind: drops the message `uuid` and everything after it from the view. */
export function truncateAt(chatId: string, uuid: string) {
  set(chatId, (s) => {
    const i = s.items.findIndex((it) => it.kind === "user" && it.uuid === uuid);
    return i < 0 ? s : { ...s, items: s.items.slice(0, i), notice: null };
  });
}

export function markInterrupting(chatId: string, on: boolean) {
  set(chatId, (s) => ({ ...s, interrupting: on }));
}

export function forgetChat(chatId: string) {
  streams.delete(chatId);
  setNeeds(chatId, null);
  setUnread(chatId, null);
  setRunState(chatId, null);
  notify(chatId);
  attached.delete(chatId);
  void detachChatChannel(chatId).catch(() => {});
}

/**
 * Subscribes to a chat. Loads it the first time; a chat whose process has stopped is
 * re-read when reopened (its session file is complete, and may have grown elsewhere).
 */
export function useChatStream(chatId: string | null): ChatStream | null {
  useEffect(() => {
    if (!chatId) return;
    void ensureChatAttached(chatId);
    const s = streams.get(chatId);
    if (!s || (s.loaded && s.state === "stopped" && s.outbox.length === 0 && !loading.has(chatId))) void load(chatId);
  }, [chatId]);

  const subscribe = useCallback(
    (l: () => void) => {
      if (!chatId) return () => {};
      let ls = listeners.get(chatId);
      if (!ls) listeners.set(chatId, (ls = new Set()));
      ls.add(l);
      return () => ls.delete(l);
    },
    [chatId],
  );
  return useSyncExternalStore(subscribe, () => (chatId ? (streams.get(chatId) ?? null) : null));
}

/** Run state per chat id (only chats that reported one; each got there via `ensureChatAttached`
 * — this hook doesn't know which ids to attach, only chat views and `seedChat` do). */
export function useRunStates(): ReadonlyMap<string, ChatRunState> {
  const subscribe = useCallback((l: () => void) => {
    stateListeners.add(l);
    return () => stateListeners.delete(l);
  }, []);
  return useSyncExternalStore(subscribe, () => runStates);
}

/**
 * Sends `text` to a chat from outside its composer (a task's first message, "ask the chat for a
 * PR"), the way the composer does: attached channel first, shown at once, streamed live.
 */
/** What "ask the chat for a PR" sends. */
export const OPEN_PR_PROMPT =
  "Open a pull request for this branch with what's already committed (don't commit anything new): push it and run " +
  "`gh pr create` against the branch it split from. Follow .github/PULL_REQUEST_TEMPLATE.md if the repo has one. Don't merge it. Reply with the PR URL.";

export async function sendToChat(chatId: string, text: string): Promise<void> {
  seedChat(chatId);
  pushOutbox(chatId, text);
  try {
    await ensureChatAttached(chatId);
    await sendChatMessage(chatId, text);
  } catch (e) {
    dropOutbox(chatId, text);
    throw e;
  }
}
