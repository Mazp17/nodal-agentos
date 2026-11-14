// Pure helpers of the Chat page: pill options, how the conversation is grouped into
// turns, the `propose_task` card and the context ring.

import { relUnder } from "../../domain/hooks/projects";
import { PROPOSE_TASK_TOOL, type ExecutorInfo, type NewTask, type PermissionRequest } from "../../domain/api";
import type { Chat, Priority, Repo, TaskStatus } from "../../domain/types";
import { formatTokens } from "../../lib/format";
import type { AgentResultInfo, SubagentRef, TranscriptItem } from "./types";

export type ToolUse = Extract<TranscriptItem, { kind: "toolUse" }>;

export interface PillOption {
  value: string | null;
  label: string;
  hint?: string;
}

// Values from `PERMISSION_MODES`, `MODEL_ALIASES` and `EFFORTS` in `runs/options.rs`.
// Chats don't offer `bypassPermissions` (repos still do, in `ProjectSettings.tsx`).
// `auto` needs sonnet or opus: with haiku Claude Code silently falls back to asking
// (verified with 2.1.283).
export const MODES: PillOption[] = [
  { value: "auto", label: "Auto", hint: "Runs safe actions, blocks risky ones · sonnet or opus" },
  { value: "acceptEdits", label: "Accept edits", hint: "Edits files, asks before commands" },
  { value: null, label: "Ask", hint: "Asks before edits and commands" },
  { value: "plan", label: "Plan", hint: "Read-only, proposes a plan" },
];

/** Modes for a chat's pill; a chat saved before Bypass was dropped keeps showing it. */
export const modeOptions = (current: string | null | undefined): PillOption[] =>
  current === "bypassPermissions" ? [...MODES, { value: current, label: "Bypass (legacy)", hint: "No prompts at all" }] : MODES;

const capitalize = (v: string) => v.charAt(0).toUpperCase() + v.slice(1);

/** Model options; the unset one names the model Claude Code's settings configure. */
export function modelOptions(configured: string | null | undefined): PillOption[] {
  return [
    configured
      ? { value: null, label: `${configured} (default)`, hint: "From Claude Code's settings" }
      : { value: null, label: "Default model", hint: "Claude Code's default" },
    { value: "haiku", label: "haiku", hint: "Fastest" },
    { value: "sonnet", label: "sonnet", hint: "Balanced" },
    { value: "opus", label: "opus", hint: "Deepest reasoning" },
  ];
}

/** Effort options; the unset one names the effort Claude Code's settings configure. */
export function effortOptions(configured: string | null | undefined): PillOption[] {
  return [
    configured
      ? { value: null, label: `${capitalize(configured)} (default)`, hint: "From Claude Code's settings" }
      : { value: null, label: "Default effort" },
    ...["low", "medium", "high", "xhigh", "max"].map((v) => ({ value: v, label: capitalize(v) })),
  ];
}

/** What a chat is called: Claude Code's name for the session, else its first message. */
export const chatTitle = (c: Pick<Chat, "title" | "sessionTitle">): string => c.sessionTitle ?? c.title ?? "New chat";

/** The Repo pill: the whole project (runs in the first repo) or one repo. */
export function repoOptions(repos: Repo[]): PillOption[] {
  return [
    { value: null, label: "All repos", hint: repos[0] ? `Whole project · runs in ${repos[0].name}` : "Whole project" },
    ...repos.map((r) => ({ value: r.id, label: r.name, hint: r.path })),
  ];
}

/** Plain Claude, then the folder's agents; a chosen agent that's no longer there stays listed. */
export function agentOptions(agents: ExecutorInfo[], current: string | null): PillOption[] {
  const out: PillOption[] = [{ value: null, label: "Claude", hint: "No agent" }];
  for (const a of agents) {
    if (a.executor.kind !== "agent") continue;
    out.push({ value: a.executor.name, label: a.executor.name, hint: a.description ?? undefined });
  }
  if (current && !out.some((o) => o.value === current)) {
    out.push({ value: current, label: current, hint: "Not found in this folder" });
  }
  return out;
}

export const optionLabel = (options: PillOption[], value: string | null | undefined, fallback?: string) =>
  options.find((o) => o.value === (value ?? null))?.label ?? fallback ?? value ?? "";

// ---------- Turns ----------

export type Part =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string; truncated: boolean }
  | { kind: "tools"; items: ToolUse[] }
  | { kind: "proposal"; item: ToolUse };

/** A message sent that Claude hasn't echoed yet; `queued` if sent while Claude was working. */
export type Outgoing = { text: string; queued: boolean };

export type Turn = { kind: "user"; text: string; pending: boolean; queued?: boolean; uuid: string | null } | { kind: "ai"; parts: Part[] };

export const isProposal = (item: ToolUse) => item.name === PROPOSE_TASK_TOOL;

/**
 * User messages become their own turn; everything between two of them is one answer, with
 * consecutive tool calls in one block. `outbox` are messages sent that Claude hasn't echoed yet.
 */
export function toTurns(items: TranscriptItem[], outbox: Outgoing[] = []): Turn[] {
  const turns: Turn[] = [];
  const ai = (): Part[] => {
    const last = turns[turns.length - 1];
    if (last?.kind === "ai") return last.parts;
    const parts: Part[] = [];
    turns.push({ kind: "ai", parts });
    return parts;
  };
  for (const it of items) {
    if (it.kind === "user") {
      turns.push({ kind: "user", text: it.text, pending: false, uuid: it.uuid ?? null });
    } else if (it.kind === "toolUse") {
      const parts = ai();
      const last = parts[parts.length - 1];
      if (isProposal(it)) parts.push({ kind: "proposal", item: it });
      else if (last?.kind === "tools") last.items.push(it);
      else parts.push({ kind: "tools", items: [it] });
    } else {
      ai().push(it.kind === "thinking" ? { kind: "thinking", text: it.text, truncated: it.truncated } : { kind: "text", text: it.text });
    }
  }
  for (const { text, queued } of outbox) turns.push({ kind: "user", text, pending: true, queued, uuid: null });
  return turns;
}

// ---------- Subagents ----------

/** Claude Code's delegation tool (`Task` in older versions). */
export const isAgentCall = (it: ToolUse) => it.name === "Agent" || it.name === "Task";

/** What a subagent call is for, from its input (clipped inputs may not parse). */
export function agentMeta(it: ToolUse): { type: string; description: string | null } {
  const input = parse(it.input);
  return { type: str(input?.subagent_type) ?? it.result?.agent?.agentType ?? "agent", description: str(input?.description) ?? it.summary };
}

/**
 * A user turn Claude Code injects itself, not something the user typed: a background
 * subagent's report (`<agent-message from="<agentId>">`), a task notification, or the
 * marker of a stop the user asked for (`[Request interrupted by user]`, also `… for tool use]`).
 */
export function harnessMessage(
  text: string,
): { kind: "report"; agentId: string } | { kind: "notification" } | { kind: "interrupted" } | null {
  const m = /^\s*<agent-message from="([A-Za-z0-9]+)">/.exec(text);
  if (m) return { kind: "report", agentId: m[1] };
  if (/^\s*\[Request interrupted by user/.test(text)) return { kind: "interrupted" };
  return /^\s*<task-notification>/.test(text) ? { kind: "notification" } : null;
}

export type AgentState = "running" | "ok" | "error" | "stopped";

/**
 * Where a subagent call stands. `live`: the answer is still being written. A call that
 * returned at once with a launch status (a background agent) keeps running while `live`.
 */
export function agentState(it: ToolUse, live: boolean): AgentState {
  const r = it.result;
  if (!r) return live ? "running" : "stopped";
  const status = (r.agent?.status ?? "").toLowerCase();
  if (r.isError || /fail|error/.test(status)) return "error";
  if (/kill|stop|cancel|abort/.test(status)) return "stopped";
  if (/async|running|launched/.test(status)) return live ? "running" : "ok";
  return "ok";
}

/** The subagent's latest tool call (what it's doing now). */
export function latestToolUse(items: TranscriptItem[] | undefined): ToolUse | null {
  for (let i = (items?.length ?? 0) - 1; i >= 0; i--) {
    const it = items?.[i];
    if (it?.kind === "toolUse") return it;
  }
  return null;
}

/** "12 tools · 38k tokens", or `null` when the result reported neither. */
export function agentStats(a: AgentResultInfo | null | undefined): string | null {
  const parts = [
    a?.toolUses != null && `${a.toolUses} ${a.toolUses === 1 ? "tool" : "tools"}`,
    a?.totalTokens != null && `${formatTokens(a.totalTokens)} tokens`,
  ].filter(Boolean);
  return parts.length ? parts.join(" · ") : null;
}

export interface AgentRow {
  toolUseId: string | null;
  agentId: string | null;
  type: string;
  description: string | null;
  state: AgentState;
  durationMs: number | null;
  stats: string | null;
  startedAt: number | null;
}

/**
 * The Agents tab: the session's subagents (`refs`, oldest first) joined with the conversation's
 * `Agent` calls by tool use id, newest first. Calls the session file doesn't list yet (just
 * launched) come first, since they're the latest.
 */
export function agentRows(refs: SubagentRef[], items: TranscriptItem[], live: boolean): AgentRow[] {
  const calls = new Map<string, ToolUse>();
  for (const it of items) if (it.kind === "toolUse" && isAgentCall(it) && it.id) calls.set(it.id, it);
  const known = new Set(refs.map((r) => r.toolUseId));
  const rows: AgentRow[] = [...refs].reverse().map((r) => {
    const call = r.toolUseId ? calls.get(r.toolUseId) : undefined;
    const a = call?.result?.agent;
    return {
      toolUseId: r.toolUseId,
      agentId: r.agentId,
      type: r.agentType ?? a?.agentType ?? "agent",
      description: r.description ?? (call ? agentMeta(call).description : null),
      state: call ? agentState(call, live) : r.stoppedByUser ? "stopped" : "ok",
      durationMs: a?.durationMs ?? null,
      stats: agentStats(a),
      startedAt: r.startedAt,
    };
  });
  const fresh: AgentRow[] = [...calls.values()]
    .filter((c) => !known.has(c.id))
    .reverse()
    .map((c) => ({
      toolUseId: c.id,
      agentId: c.result?.agent?.agentId ?? null,
      type: agentMeta(c).type,
      description: agentMeta(c).description,
      state: agentState(c, live),
      durationMs: c.result?.agent?.durationMs ?? null,
      stats: agentStats(c.result?.agent),
      startedAt: null,
    }));
  return [...fresh, ...rows];
}

/** `mcp__nodal__list_tasks` → `nodal · list_tasks`. */
export const toolLabel = (name: string) => name.replace(/^mcp__(.+?)__/, "$1 · ");

/** Claude Code's built-in tools as a row reads while running and once done. */
const VERBS: Record<string, [string, string]> = {
  Read: ["Reading", "Read"],
  Edit: ["Editing", "Edited"],
  MultiEdit: ["Editing", "Edited"],
  Write: ["Writing", "Wrote"],
  NotebookEdit: ["Editing", "Edited"],
  Grep: ["Searching", "Searched"],
  Glob: ["Listing", "Listed"],
  LS: ["Listing", "Listed"],
  Bash: ["Running", "Ran"],
  WebFetch: ["Fetching", "Fetched"],
  WebSearch: ["Searching the web for", "Searched the web for"],
  Task: ["Delegating", "Delegated"],
  Agent: ["Delegating", "Delegated"],
  Skill: ["Using skill", "Used skill"],
  TodoWrite: ["Updating the plan", "Updated the plan"],
};

/** `Read` → `Reading` / `Read`; other tools keep their label. */
export const toolVerb = (name: string, running: boolean): string =>
  VERBS[name]?.[running ? 0 : 1] ?? toolLabel(name);

/** A tool's argument with the repo's own path left out: `/…/core-api/src/a.ts` → `src/a.ts`. */
export function shortArg(summary: string | null, repos: Repo[]): string {
  if (!summary) return "";
  for (const r of [...repos].sort((a, b) => b.path.length - a.path.length)) {
    const rel = relUnder(summary, r.path);
    if (rel !== null) return rel;
  }
  return summary;
}

export type PatchFile = NonNullable<NonNullable<ToolUse["result"]>["patch"]>["file"];
export interface TurnChanges {
  files: number;
  additions: number;
  deletions: number;
  /** Per file, in first-edit order, with every edit's diff in order. */
  byFile: { path: string; additions: number; deletions: number; edits: PatchFile[] }[];
}

/**
 * Files an answer changed, from the diffs of its edits; `null` when it changed none. Lines add
 * up per edit, so a line edited twice counts twice.
 */
export function turnChanges(parts: Part[]): TurnChanges | null {
  const files = new Map<string, TurnChanges["byFile"][number]>();
  for (const p of parts) {
    if (p.kind !== "tools") continue;
    for (const it of p.items) {
      const f = it.result && !it.result.isError ? it.result.patch?.file : null;
      if (!f) continue;
      const prev = files.get(f.path) ?? { path: f.path, additions: 0, deletions: 0, edits: [] };
      files.set(f.path, { ...prev, additions: prev.additions + f.additions, deletions: prev.deletions + f.deletions, edits: [...prev.edits, f] });
    }
  }
  if (files.size === 0) return null;
  const byFile = [...files.values()];
  return {
    files: byFile.length,
    additions: byFile.reduce((n, f) => n + f.additions, 0),
    deletions: byFile.reduce((n, f) => n + f.deletions, 0),
    byFile,
  };
}

/**
 * What the user answered to an `AskUserQuestion` call: Claude Code's result reads
 * `User has answered your questions: "<question>"="<answer>", …`. Headers come from the input
 * when it parses. `null` when the call has no usable result.
 */
export function readAnswered(it: ToolUse): { header: string | null; answer: string }[] | null {
  if (it.name !== "AskUserQuestion" || !it.result || it.result.isError) return null;
  const qs = parse(it.input)?.questions;
  const header = (q: string) => {
    const f = Array.isArray(qs) ? (qs as Record<string, unknown>[]).find((x) => x?.question === q) : null;
    return str(f?.header);
  };
  const out = [...it.result.text.matchAll(/"([^"]+)"\s*=\s*"([^"]*)"/g)].map((m) => ({ header: header(m[1]), answer: m[2] }));
  return out.length ? out : null;
}

/** Short note next to a finished tool call: its line count or its first line. */
export function resultMeta(text: string): string {
  const lines = text.split("\n").filter((l) => l.trim());
  if (lines.length > 1) return `${lines.length} lines`;
  const one = lines[0]?.trim() ?? "";
  return one.length > 40 ? `${one.slice(0, 39)}…` : one;
}

// ---------- propose_task ----------

export interface Proposal {
  newTask: NewTask;
  repoName: string;
}

export type ProposalState =
  | { status: "pending" }
  | { status: "error"; message: string }
  | { status: "ok"; proposal: Proposal }
  /** Neither the result nor the input could be read back (both are clipped when long). */
  | { status: "unreadable"; title: string | null };

const PRIORITIES: readonly Priority[] = ["urgent", "high", "medium", "low", "none"];
const STATUSES: readonly TaskStatus[] = ["backlog", "todo", "in_progress", "in_review", "blocked", "done", "canceled"];

const parse = (s: string | null | undefined): Record<string, unknown> | null => {
  if (!s) return null;
  try {
    const v: unknown = JSON.parse(s);
    return v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : null;
  } catch {
    return null;
  }
};
const str = (v: unknown) => (typeof v === "string" ? v : null);
const strs = (v: unknown) => (Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && !!x.trim()) : []);

/** Like `resolve_repo` in `mcp/tools.rs`: id, name (any case) or a path inside the repo. */
export function findRepo(repos: Repo[], s: string): Repo | null {
  const q = s.trim();
  const byName = repos.filter((r) => r.name.toLowerCase() === q.toLowerCase());
  return (
    repos.find((r) => r.id === q) ??
    (byName.length === 1 ? byName[0] : null) ??
    [...repos].sort((a, b) => b.path.length - a.path.length).find((r) => q === r.path || q.startsWith(`${r.path}/`)) ??
    null
  );
}

/**
 * The card of a `propose_task` call: from its result (the `NewTask` the tool validated) or,
 * when that was clipped, from the call's own input.
 */
export function readProposal(item: ToolUse, projectId: string, repos: Repo[]): ProposalState {
  const r = item.result;
  if (!r) return { status: "pending" };
  if (r.isError) return { status: "error", message: r.text || "The proposal was rejected." };
  const out = parse(r.text);
  const nt = out?.newTask as Record<string, unknown> | undefined;
  if (nt && str(nt.title) && str(nt.repoId) && str(nt.projectId)) {
    const repoName = str(out?.repoName) ?? repos.find((x) => x.id === nt.repoId)?.name ?? "";
    return { status: "ok", proposal: { newTask: nt as unknown as NewTask, repoName } };
  }
  const input = parse(item.input);
  const title = str(input?.title);
  const plan = str(input?.plan);
  const repoArg = str(input?.repo);
  const repo = repoArg ? findRepo(repos, repoArg) : null;
  if (!input || !title || !plan || !repo) return { status: "unreadable", title: title ?? item.summary };
  const priority = str(input.priority) as Priority | null;
  const status = str(input.status) as TaskStatus | null;
  return {
    status: "ok",
    proposal: {
      repoName: repo.name,
      newTask: {
        projectId,
        repoId: repo.id,
        title: title.split(/\s+/).filter(Boolean).join(" "),
        plan: { kind: "text", text: plan },
        status: status && STATUSES.includes(status) ? status : "todo",
        priority: priority && PRIORITIES.includes(priority) ? priority : "none",
        labels: strs(input.labels),
        acceptance: strs(input.acceptance).map((a) => a.trim()),
      },
    },
  };
}

// ---------- Context ring ----------

/** Claude's usual context window, until a turn reports the real one. */
export const DEFAULT_CONTEXT_WINDOW = 200_000;

/** Percent of the context window in use (0–100), or `null` before the first answer. */
export function contextPercent(tokens: number | null, window: number | null): number | null {
  if (tokens == null) return null;
  const w = window && window > 0 ? window : DEFAULT_CONTEXT_WINDOW;
  return Math.max(0, Math.min(100, Math.round((tokens / w) * 100)));
}

/** Sessions list: `now`, `5m`, `3h`, `2d`. */
export function shortAgo(ms: number, now: number): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  if (s < 60) return "now";
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

/** One question of Claude's `AskUserQuestion` tool. */
export interface Question {
  question: string;
  header: string | null;
  multiSelect: boolean;
  options: { label: string; description: string }[];
}

/** The questions of an `AskUserQuestion` request, or null for any other prompt (or an unreadable one). */
export function readQuestions(req: PermissionRequest): Question[] | null {
  if (req.toolName !== "AskUserQuestion") return null;
  const raw = (req.input as { questions?: unknown } | null)?.questions;
  if (!Array.isArray(raw) || raw.length === 0) return null;
  const out: Question[] = [];
  for (const q of raw as Record<string, unknown>[]) {
    const question = str(q?.question);
    if (!question || !Array.isArray(q.options)) return null;
    const options = (q.options as Record<string, unknown>[])
      .map((o) => ({ label: str(o?.label) ?? "", description: str(o?.description) ?? "" }))
      .filter((o) => o.label);
    out.push({ question, header: str(q.header), multiSelect: q.multiSelect === true, options });
  }
  return out;
}
