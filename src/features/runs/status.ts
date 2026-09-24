// Estado derivado de un run para pintar: cruza la fila de Nodal (`Run`) con lo que dice
// Claude Code (`claude agents` y el detalle del workflow). Puro: sin hooks ni invoke.

import { taskKey, type Executor, type Project, type RunLight, type RunOutcome, type Task } from "../../domain/types";
import type { RunDetail, RunSummary } from "./types";

export type BadgeTone = "accent" | "ok" | "warn" | "danger" | "muted";

/**
 * - `awaiting`: run migrado que quedó en cola; espera `confirm_run`.
 * - `starting`: lanzado pero todavía no aparece en `claude agents`.
 * - `waiting`: la sesión espera al usuario (permiso, input).
 */
export type RunPhase =
  | "awaiting"
  | "queued"
  | "launching"
  | "starting"
  | "running"
  | "waiting"
  | "finished"
  | "failed"
  | "canceled";

export type RunsTab = "active" | "queued" | "finished" | "failed";

export interface RunView {
  run: RunLight;
  /** Sesión en `claude agents`, si ya figura. */
  live: RunSummary | null;
  /** Detalle del workflow: `undefined` sin pedir aún, `null` si la sesión no corrió uno. */
  detail: RunDetail | null | undefined;
  phase: RunPhase;
  tab: RunsTab;
  label: string;
  tone: BadgeTone;
  /** Algo en marcha: el punto late. */
  pulse: boolean;
  /** "permission prompt", "input needed"... si `phase === "waiting"`. */
  waitingFor: string | null;
  /** 1-based en la cola global (solo `queued`/`awaiting`). */
  queuePos: number | null;
  phaseIndex: number | null;
  phaseTotal: number;
  phaseName: string | null;
  tokens: number | null;
  durationMs: number | null;
  /** Revisor lanzado sobre este run (el más reciente), si lo hay. */
  review: RunLight | null;
}

/** Espejo de `LAUNCH_GRACE_MS` en src-tauri/src/work/queue.rs. */
export const LAUNCH_GRACE_MS = 90_000;

/** En cola, lanzándose o lanzado: la tarea tiene trabajo en curso. */
export const isActive = (run: RunLight) => run.status === "queued" || run.status === "launching" || run.status === "launched";

/** Run migrado en cola que no se lanza hasta confirmarlo (`confirm_run`). */
export const awaitingConfirmation = (run: RunLight) => run.status === "queued" && run.legacyLabel !== null;

export function executorLabel(e: Executor): string {
  switch (e.kind) {
    case "agent":
      return e.name;
    case "workflow":
      return e.name;
    case "claude":
      return "Claude";
  }
}

export function executorKindLabel(e: Executor): string {
  return e.kind === "agent" ? "Agent" : e.kind === "workflow" ? "Workflow" : "Session";
}

export const OUTCOME_LABEL: Record<RunOutcome, string> = {
  green: "Green",
  yellow: "Yellow",
  red: "Red",
  stopped: "Stopped",
  unknown: "No result",
};

export const OUTCOME_TONE: Record<RunOutcome, BadgeTone> = {
  green: "ok",
  yellow: "warn",
  red: "danger",
  stopped: "danger",
  unknown: "muted",
};

/** "#482" para URLs de PR de GitHub; si no, `null`. */
export function prNumber(url: string | null | undefined): string | null {
  const m = url ? /\/pull\/(\d+)/.exec(url) : null;
  return m ? `#${m[1]}` : null;
}

const WORKSPACE_TRUST_RE = /Workspace not trusted/i;

function waitingLabel(w: string | null): string {
  return w === "permission prompt" ? "Needs permission" : "Needs input";
}

function finishedLabel(run: RunLight): { label: string; tone: BadgeTone } {
  if (run.kind === "review" && run.verdict) {
    return run.verdict.pass ? { label: "Review passed", tone: "ok" } : { label: "Review failed", tone: "danger" };
  }
  const o = run.outcome ?? "unknown";
  const base = OUTCOME_LABEL[o];
  const pr = prNumber(run.prUrl);
  if (pr && (o === "green" || o === "yellow")) return { label: `PR ${pr} · ${base}`, tone: OUTCOME_TONE[o] };
  return { label: base, tone: OUTCOME_TONE[o] };
}

export interface DeriveContext {
  live: RunSummary[];
  details: Record<string, RunDetail | null>;
  /** Id de run → posición 1-based en la cola global. */
  queuePos: Map<string, number>;
  /** Id de run de trabajo → su revisor más reciente. */
  reviews: Map<string, RunLight>;
  now: number;
}

export function findLive(run: RunLight, live: RunSummary[]): RunSummary | null {
  if (!run.claudeRunId && !run.sessionId) return null;
  return live.find((s) => (run.claudeRunId && s.id === run.claudeRunId) || (run.sessionId && s.sessionId === run.sessionId)) ?? null;
}

export function deriveRunView(run: RunLight, ctx: DeriveContext): RunView {
  const live = findLive(run, ctx.live);
  const detail = ctx.details[run.id];
  const queuePos = ctx.queuePos.get(run.id) ?? null;
  let phase: RunPhase;
  switch (run.status) {
    case "queued":
      phase = awaitingConfirmation(run) ? "awaiting" : "queued";
      break;
    case "launching":
      phase = "launching";
      break;
    case "launched":
      if (!live) phase = "starting";
      else if (live.state === "blocked") phase = "waiting";
      else phase = "running";
      break;
    case "finished":
      phase = "finished";
      break;
    case "failed":
      phase = "failed";
      break;
    case "canceled":
      phase = "canceled";
      break;
  }

  const phaseIndex = detail?.currentPhaseIndex ?? null;
  const phaseTotal = detail?.phases.length ?? 0;
  const phaseName = detail?.currentPhase ?? null;
  const waitingFor = phase === "waiting" ? (live?.waitingFor ?? "input needed") : null;

  let label: string;
  let tone: BadgeTone;
  switch (phase) {
    case "awaiting":
      [label, tone] = ["Awaiting confirmation", "warn"];
      break;
    case "queued":
      [label, tone] = [queuePos != null ? `Queued · #${queuePos}` : "Queued", "muted"];
      break;
    case "launching":
      [label, tone] = ["Launching", "accent"];
      break;
    case "starting": {
      const fresh = run.launchedAt == null || ctx.now - run.launchedAt < LAUNCH_GRACE_MS;
      [label, tone] = fresh ? ["Starting", "accent"] : ["Not visible", "warn"];
      break;
    }
    case "running":
      if (run.kind === "review") label = "Reviewing";
      else if (phaseIndex != null && phaseTotal) label = `Phase ${phaseIndex}/${phaseTotal}${phaseName ? ` · ${phaseName}` : ""}`;
      else label = "Running";
      tone = "accent";
      break;
    case "waiting":
      [label, tone] = [waitingLabel(waitingFor), "warn"];
      break;
    case "finished":
      ({ label, tone } = finishedLabel(run));
      break;
    case "failed":
      [label, tone] = [run.launchedAt == null || WORKSPACE_TRUST_RE.test(run.error ?? "") ? "Launch failed" : "Failed", "danger"];
      break;
    case "canceled":
      [label, tone] = run.outcome === "stopped" ? ["Stopped", "danger"] : ["Canceled", "muted"];
      break;
  }

  const tab: RunsTab =
    phase === "awaiting" || phase === "queued"
      ? "queued"
      : phase === "finished"
        ? "finished"
        : phase === "failed" || phase === "canceled"
          ? "failed"
          : "active";

  const active = tab === "active";
  let durationMs: number | null = null;
  if (run.launchedAt != null) {
    if (active) durationMs = ctx.now - run.launchedAt;
    else if (detail?.durationMs != null) durationMs = detail.durationMs;
    else if (run.finishedAt != null) durationMs = Math.max(0, run.finishedAt - run.launchedAt);
  }

  return {
    run,
    live,
    detail,
    phase,
    tab,
    label,
    tone,
    pulse: phase === "running" || phase === "launching" || phase === "starting",
    waitingFor,
    queuePos,
    phaseIndex,
    phaseTotal,
    phaseName,
    tokens: detail?.totalTokens ?? null,
    durationMs,
    review: ctx.reviews.get(run.id) ?? null,
  };
}

/** Texto corto del estado ("Phase 3/9 · Implement", "Needs permission", "PR #12 · Green"). */
export const runStatusLabel = (v: RunView) => v.label;

/** Texto para la columna Phase de la tabla y su porcentaje de avance. */
export function phaseProgress(v: RunView): { text: string; pct: number } {
  const n = v.phaseTotal;
  switch (v.phase) {
    case "awaiting":
    case "queued":
      return { text: "Waiting", pct: 0 };
    case "launching":
    case "starting":
      return { text: "Starting", pct: 0 };
    case "waiting":
    case "running":
      if (v.phaseIndex != null && n) {
        return { text: `${v.phaseIndex}/${n}${v.phaseName ? ` · ${v.phaseName}` : ""}`, pct: Math.round((v.phaseIndex / n) * 100) };
      }
      return { text: v.run.kind === "review" ? "Reviewing" : "Running", pct: 0 };
    case "finished":
      return { text: n ? `${n}/${n}${v.phaseName ? ` · ${v.phaseName}` : ""}` : "Done", pct: 100 };
    case "failed":
    case "canceled":
      if (v.run.launchedAt == null) return { text: "Not started", pct: 0 };
      if (v.phaseIndex != null && n) {
        return { text: `Stopped · ${v.phaseName ?? v.phaseIndex}`, pct: Math.round((v.phaseIndex / n) * 100) };
      }
      return { text: "Stopped", pct: 0 };
  }
}

/** Id visible de la tarea del run (`PAY-12`) y su título; sin tarea, la etiqueta migrada. */
export function runTaskRef(run: RunLight, task: Task | undefined, project: Project | undefined): { key: string | null; title: string } {
  if (task) return { key: project ? taskKey(project.key, task.number) : null, title: task.title };
  if (run.legacyLabel) return { key: null, title: run.legacyLabel };
  return { key: null, title: run.taskId ? "(deleted task)" : `${executorLabel(run.executor)} run` };
}

/** Nombre corto para toasts y confirmaciones. */
export function runName(run: RunLight, task: Task | undefined, project: Project | undefined): string {
  const ref = runTaskRef(run, task, project);
  return ref.key ?? ref.title;
}
