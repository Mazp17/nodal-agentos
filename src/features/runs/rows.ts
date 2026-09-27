// Rows of the Runs screen: Nodal's runs and the sessions started outside Nodal, in one
// shape, grouped by what they need. Pure: no hooks or invoke.

import type { Executor, Project, Repo, Task } from "../../domain/types";
import { formatDuration, formatTokens } from "../../lib/format";
import type { ExternalSessions, SessionActivity } from "./external";
import { executorKindLabel, executorLabel, phaseProgress, runTaskRef, type BadgeTone, type RunView } from "./status";

export type RunGroup = "needs" | "running" | "queued" | "done";
export type RunSource = "all" | "nodal" | "external";

export interface RunRow {
  id: string;
  group: RunGroup;
  projectId: string | null;
  repoId: string | null;
  /** Task key (`PAY-12`); `null` for external sessions and runs without a task. */
  taskKey: string | null;
  title: string;
  project: Project | undefined;
  repoName: string;
  /** `null`: external session (dashed avatar). */
  executor: Executor | null;
  by: string;
  byTitle: string;
  /** Progress bar in 0–100; `null`: text only (external sessions). */
  pct: number | null;
  progress: string;
  meta: string;
  launchedAt: number | null;
  /** Sort key within the group (queued rows use `queuePos`). */
  at: number;
  label: string;
  tone: BadgeTone | "faint";
  pulse: boolean;
  /** An external session is editing the same repo while this run works. */
  clash: string | null;
  /** Only Nodal runs. */
  view: RunView | null;
}

export const GROUPS: { id: RunGroup; label: string; tone: BadgeTone; hint?: string }[] = [
  { id: "needs", label: "Needs you", tone: "warn" },
  { id: "running", label: "Running", tone: "accent" },
  { id: "queued", label: "Queued", tone: "muted", hint: "Starts in order when a slot frees up" },
  { id: "done", label: "Finished", tone: "ok" },
];

function groupOf(v: RunView, latestByTask: Map<string, RunView>): RunGroup {
  switch (v.phase) {
    case "waiting":
      return "needs";
    case "failed":
      // Only the task's latest attempt needs you; older failures were already retried.
      return !v.run.taskId || latestByTask.get(v.run.taskId)?.run.id === v.run.id ? "needs" : "done";
    case "awaiting":
    case "queued":
      return "queued";
    case "launching":
    case "starting":
    case "running":
      return "running";
    case "finished":
    case "canceled":
      return "done";
  }
}

const ORIGIN: Record<string, string> = { interactive: "Terminal", unlisted: "Script · claude -p", background: "Background" };
const VERB: Record<string, string> = {
  Edit: "Editing",
  MultiEdit: "Editing",
  Write: "Writing",
  NotebookEdit: "Editing",
  Read: "Reading",
  Bash: "Running",
  Grep: "Searching",
  Glob: "Listing",
};
const WRITING_TOOLS = new Set(["Edit", "MultiEdit", "Write", "NotebookEdit", "Bash"]);

const basename = (p: string | null) => (p ? p.replace(/\/+$/, "").split("/").pop() || p : null);
const sessionTitle = (s: SessionActivity) => s.name ?? basename(s.cwd) ?? "Claude session";
const doing = (s: SessionActivity) =>
  s.lastTool ? [VERB[s.lastTool] ?? s.lastTool, s.lastToolSummary].filter(Boolean).join(" ") : s.alive ? "Idle" : "—";
const isWaiting = (s: SessionActivity) => s.state === "blocked" || s.status === "waiting";
const isBusy = (s: SessionActivity) => s.state === "working" || s.status === "busy";

function sessionStatus(s: SessionActivity): { label: string; tone: RunRow["tone"]; pulse: boolean; group: RunGroup } {
  if (!s.alive) return { label: s.state === "stopped" ? "Stopped" : "Ended", tone: "faint", pulse: false, group: "done" };
  if (isWaiting(s)) return { label: s.waitingFor ? `Waiting · ${s.waitingFor}` : "Waiting", tone: "warn", pulse: false, group: "needs" };
  if (isBusy(s)) return { label: "Working", tone: "accent", pulse: true, group: "running" };
  return { label: "Idle", tone: "muted", pulse: false, group: "running" };
}

export interface RowContext {
  projects: Map<string, Project>;
  repos: Map<string, Repo>;
  tasks: Map<string, Task>;
  latestByTask: Map<string, RunView>;
  now: number;
}

function externalRows(ext: ExternalSessions | undefined, ctx: RowContext): RunRow[] {
  return (ext?.repos ?? []).flatMap(({ repoId, sessions, subagents }) => {
    const repo = ctx.repos.get(repoId);
    const project = repo ? ctx.projects.get(repo.projectId) : undefined;
    // `external_sessions` also covers archived projects' repos.
    if (project?.archivedAt != null) return [];
    return sessions.map((s): RunRow => {
      const st = sessionStatus(s);
      const subs = subagents.filter((a) => a.sessionId === s.sessionId).length;
      const origin = ORIGIN[s.kind] ?? "Session";
      const end = s.alive ? ctx.now : (s.lastActivityAt ?? ctx.now);
      const dur = s.startedAt != null ? end - s.startedAt : null;
      return {
        id: `ext:${s.sessionId}`,
        group: st.group,
        projectId: repo?.projectId ?? null,
        repoId,
        taskKey: null,
        title: sessionTitle(s),
        project,
        repoName: repo?.name ?? "—",
        executor: null,
        by: origin,
        byTitle: `External session · started outside Nodal · read-only${subs ? ` · ${subs} subagent${subs === 1 ? "" : "s"}` : ""}`,
        pct: null,
        progress: doing(s),
        meta: dur != null ? formatDuration(dur) : "",
        launchedAt: s.startedAt,
        at: s.startedAt ?? s.lastActivityAt ?? 0,
        label: st.label,
        tone: st.tone,
        pulse: st.pulse,
        clash: null,
        view: null,
      };
    });
  });
}

/** A live external session running a writing tool in `repoId`, if any. */
function clashIn(ext: ExternalSessions | undefined, repoId: string | null): SessionActivity | null {
  if (!repoId) return null;
  const r = ext?.repos.find((x) => x.repoId === repoId);
  return r?.sessions.find((s) => s.alive && isBusy(s) && s.lastTool != null && WRITING_TOOLS.has(s.lastTool)) ?? null;
}

function nodalRow(v: RunView, ext: ExternalSessions | undefined, ctx: RowContext): RunRow {
  const task = v.run.taskId ? ctx.tasks.get(v.run.taskId) : undefined;
  const repoId = v.run.repoId ?? task?.repoId ?? null;
  const repo = repoId ? ctx.repos.get(repoId) : undefined;
  const projectId = task?.projectId ?? repo?.projectId ?? null;
  const project = projectId ? ctx.projects.get(projectId) : undefined;
  const ref = runTaskRef(v.run, task, project);
  const ph = phaseProgress(v);
  const group = groupOf(v, ctx.latestByTask);
  const review = v.run.kind === "review";
  const kind = review ? "Reviewer" : executorKindLabel(v.run.executor);
  const cx = group === "running" || v.phase === "waiting" ? clashIn(ext, repoId) : null;
  const meta = [group === "queued" ? "" : formatDuration(v.durationMs), v.tokens != null ? `${formatTokens(v.tokens)} tokens` : ""];
  return {
    id: v.run.id,
    group,
    projectId,
    repoId,
    taskKey: ref.key,
    title: ref.title,
    project,
    repoName: repo?.name ?? "—",
    executor: v.run.executor,
    by: executorLabel(v.run.executor),
    byTitle: `${kind} ${executorLabel(v.run.executor)}`,
    pct: ph.pct,
    progress: ph.text,
    meta: meta.filter((x) => x && x !== "—").join(" · "),
    launchedAt: v.run.launchedAt,
    at: group === "queued" ? (v.queuePos ?? 0) : (v.run.launchedAt ?? v.run.queuedAt),
    label: v.phase === "running" ? (review ? "Reviewing" : "Running") : v.label,
    tone: v.tone,
    pulse: v.pulse,
    clash: cx ? `“${sessionTitle(cx)}” is ${doing(cx).toLowerCase()}. Changes may conflict.` : null,
    view: v,
  };
}

export function buildRows(views: RunView[], ext: ExternalSessions | undefined, source: RunSource, ctx: RowContext): RunRow[] {
  const nodal = source === "external" ? [] : views.map((v) => nodalRow(v, ext, ctx));
  const outside = source === "nodal" ? [] : externalRows(ext, ctx);
  return [...nodal, ...outside];
}

/** Rows of one group: the queue in launch order, the rest most recent first. */
export function sortGroup(rows: RunRow[], group: RunGroup): RunRow[] {
  return [...rows].sort((a, b) => (group === "queued" ? a.at - b.at : b.at - a.at));
}

/** "Today 14:32", "Yesterday 09:05", "Sep 3, 18:40". */
export function formatLaunch(ms: number, now: number): string {
  const d = new Date(ms);
  const hm = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
  const day = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const diff = Math.round((day(new Date(now)) - day(d)) / 864e5);
  if (diff === 0) return `Today ${hm}`;
  if (diff === 1) return `Yesterday ${hm}`;
  return `${d.toLocaleDateString([], { month: "short", day: "numeric" })}, ${hm}`;
}
