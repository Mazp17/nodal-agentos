// Live state of each chat: its history (read once from the Claude Code session) plus what
// `nodal://chat` streams after that. One listener for the whole app; views subscribe per chat.

import { useCallback, useEffect, useSyncExternalStore } from "react";
import {
  getChatLive,
  getChatTranscript,
  onChatEvent,
  type ChatEvent,
  type ChatRunState,
  type PermissionRequest,
} from "../../domain/api";
import type { TranscriptItem } from "../runs/types";

export interface ChatStream {
  /** `false` while the history loads. */
  loaded: boolean;
  loadError: string | null;
  /** Older items the history left out. */
  omitted: number;
  /** History, then every complete item streamed since. */
  items: TranscriptItem[];
  /** Messages sent that Claude hasn't echoed back yet. */
  outbox: string[];
  /** Text of the answer being written (a later `item` carries it in full). */
  streaming: string;
  thinking: boolean;
  state: ChatRunState;
  /** Permission requests waiting for an answer, oldest first. */
  pending: PermissionRequest[];
  /** Tool calls denied without asking, by tool use id. */
  denied: Record<string, string>;
  contextTokens: number | null;
  contextWindow: number | null;
  /** Why the last turn failed, until the next message. */
  notice: string | null;
  interrupting: boolean;
}

const EMPTY: ChatStream = {
  loaded: false,
  loadError: null,
  omitted: 0,
  items: [],
  outbox: [],
  streaming: "",
  thinking: false,
  state: "stopped",
  pending: [],
  denied: {},
  contextTokens: null,
  contextWindow: null,
  notice: null,
  interrupting: false,
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
  streams.set(chatId, update(cur));
  notify(chatId);
}

function setRunState(chatId: string, state: ChatRunState | null) {
  if ((runStates.get(chatId) ?? null) === state) return;
  const next = new Map(runStates);
  if (state) next.set(chatId, state);
  else next.delete(chatId);
  runStates = next;
  stateListeners.forEach((l) => l());
}

const withResult = (items: TranscriptItem[], id: string, apply: (t: Extract<TranscriptItem, { kind: "toolUse" }>) => TranscriptItem) => {
  const i = items.findIndex((it) => it.kind === "toolUse" && it.id === id);
  if (i < 0) return items;
  const next = [...items];
  next[i] = apply(items[i] as Extract<TranscriptItem, { kind: "toolUse" }>);
  return next;
};

/** Applies one streamed event (exported for tests of the reducer). */
export function reduce(s: ChatStream, e: ChatEvent): ChatStream {
  switch (e.type) {
    case "init":
      return s;
    case "textDelta":
      return { ...s, streaming: s.streaming + e.text, thinking: false };
    case "thinkingDelta":
      return { ...s, thinking: true };
    case "item": {
      const it = e.item;
      if (it.kind === "user") return { ...s, items: [...s.items, it], outbox: s.outbox.slice(1) };
      // Replayed after a reload of the history: keep the one already there.
      if (it.kind === "toolUse" && it.id && s.items.some((x) => x.kind === "toolUse" && x.id === it.id)) return s;
      return { ...s, items: [...s.items, it], streaming: it.kind === "text" ? "" : s.streaming, thinking: false };
    }
    case "toolResult":
      return { ...s, items: withResult(s.items, e.toolUseId, (t) => ({ ...t, result: e.result })) };
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
      const notice = e.ok ? null : s.interrupting ? "Stopped." : (e.result ?? `The answer ended early (${e.subtype}).`);
      return {
        ...s,
        streaming: "",
        thinking: false,
        interrupting: false,
        notice,
        contextWindow: e.contextWindow ?? s.contextWindow,
      };
    }
    case "state":
      return e.state === "busy"
        ? { ...s, state: e.state }
        : { ...s, state: e.state, streaming: "", thinking: false, interrupting: false, ...(e.state === "stopped" ? { pending: [] } : {}) };
    case "error":
      return { ...s, notice: e.message, outbox: [], streaming: "", thinking: false, interrupting: false };
  }
}

let unlisten: Promise<() => void> | null = null;

function listen() {
  if (unlisten || typeof window === "undefined" || !("__TAURI_INTERNALS__" in window)) return;
  unlisten = onChatEvent(({ chatId, event }) => {
    if (event.type === "state") setRunState(chatId, event.state);
    if (streams.has(chatId)) set(chatId, (s) => reduce(s, event));
  });
  unlisten.catch((err: unknown) => console.error("nodal://chat", err));
  import.meta.hot?.dispose(() => void unlisten?.then((u) => u(), () => {}));
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
  listen();
  if (!streams.has(chatId)) void load(chatId, true);
}

/** Before sending: shows the message until Claude echoes it. */
export function pushOutbox(chatId: string, text: string) {
  set(chatId, (s) => ({ ...s, outbox: [...s.outbox, text], notice: null }));
}

export function dropOutbox(chatId: string, text: string) {
  set(chatId, (s) => {
    const i = s.outbox.lastIndexOf(text);
    return i < 0 ? s : { ...s, outbox: s.outbox.filter((_, j) => j !== i) };
  });
}

export function markInterrupting(chatId: string, on: boolean) {
  set(chatId, (s) => ({ ...s, interrupting: on }));
}

export function forgetChat(chatId: string) {
  streams.delete(chatId);
  setRunState(chatId, null);
  notify(chatId);
}

/**
 * Subscribes to a chat. Loads it the first time; a chat whose process has stopped is
 * re-read when reopened (its session file is complete, and may have grown elsewhere).
 */
export function useChatStream(chatId: string | null): ChatStream | null {
  useEffect(() => {
    listen();
    if (!chatId) return;
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

/** Run state per chat id (only chats that reported one). */
export function useRunStates(): ReadonlyMap<string, ChatRunState> {
  useEffect(listen, []);
  const subscribe = useCallback((l: () => void) => {
    stateListeners.add(l);
    return () => stateListeners.delete(l);
  }, []);
  return useSyncExternalStore(subscribe, () => runStates);
}
