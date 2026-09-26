// Pure helpers of the Chat page: pill options, how the conversation is grouped into
// turns, the `propose_task` card and the context ring.

import { PROPOSE_TASK_TOOL, type NewTask } from "../../domain/api";
import type { Executor, Priority, Repo, TaskStatus } from "../../domain/types";
import type { TranscriptItem } from "../runs/types";

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

export const MODELS: PillOption[] = [
  { value: null, label: "Default model", hint: "Claude Code's default" },
  { value: "haiku", label: "haiku", hint: "Fastest" },
  { value: "sonnet", label: "sonnet", hint: "Balanced" },
  { value: "opus", label: "opus", hint: "Deepest reasoning" },
];
export const EFFORTS: PillOption[] = [
  { value: null, label: "Default effort" },
  ...["low", "medium", "high", "xhigh", "max"].map((v) => ({ value: v, label: capitalize(v) })),
];

/** The Repo pill: the whole project (runs in the first repo) or one repo. */
export function repoOptions(repos: Repo[]): PillOption[] {
  return [
    { value: null, label: "All repos", hint: repos[0] ? `Whole project · runs in ${repos[0].name}` : "Whole project" },
    ...repos.map((r) => ({ value: r.id, label: r.name, hint: r.path })),
  ];
}

export const optionLabel = (options: PillOption[], value: string | null | undefined, fallback?: string) =>
  options.find((o) => o.value === (value ?? null))?.label ?? fallback ?? value ?? "";

// ---------- Turns ----------

export type Part =
  | { kind: "text"; text: string }
  | { kind: "thinking"; text: string }
  | { kind: "tools"; items: ToolUse[] }
  | { kind: "proposal"; item: ToolUse };

export type Turn = { kind: "user"; text: string; pending: boolean } | { kind: "ai"; parts: Part[] };

export const isProposal = (item: ToolUse) => item.name === PROPOSE_TASK_TOOL;

/**
 * User messages become their own turn; everything between two of them is one answer, with
 * consecutive tool calls in one block. `outbox` are messages sent that Claude hasn't echoed yet.
 */
export function toTurns(items: TranscriptItem[], outbox: string[] = []): Turn[] {
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
      turns.push({ kind: "user", text: it.text, pending: false });
    } else if (it.kind === "toolUse") {
      const parts = ai();
      const last = parts[parts.length - 1];
      if (isProposal(it)) parts.push({ kind: "proposal", item: it });
      else if (last?.kind === "tools") last.items.push(it);
      else parts.push({ kind: "tools", items: [it] });
    } else {
      ai().push({ kind: it.kind, text: it.text });
    }
  }
  for (const text of outbox) turns.push({ kind: "user", text, pending: true });
  return turns;
}

/** `mcp__nodal__list_tasks` → `nodal · list_tasks`. */
export const toolLabel = (name: string) => name.replace(/^mcp__(.+?)__/, "$1 · ");

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
  const executor = input.executor && typeof input.executor === "object" ? (input.executor as Executor) : null;
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
        assignee: executor,
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
