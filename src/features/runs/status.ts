import type { IssueRun, RunDetail, RunSummary } from "./types";

export type BadgeTone = "accent" | "ok" | "warn" | "danger" | "muted";

/** Estado derivado para pintar: agrupa lo que dicen `IssueRun`, `claude agents` y el detalle. */
export type RunKind = "queued" | "starting" | "running" | "done" | "failed";

export interface RunBadge {
  label: string;
  tone: BadgeTone;
  title: string;
  /** Hay un run en marcha o en cola: no se puede lanzar otro para la issue. */
  active: boolean;
}

export interface RunView extends RunBadge {
  /** Id estable para navegar: `i:<issueId>:<queuedAt>` o `s:<sessionId>`. */
  key: string;
  kind: RunKind;
  ir: IssueRun | null;
  run: RunSummary | undefined;
  detail: RunDetail | null | undefined;
  issueId: string | null;
  identifier: string | null;
  workflow: string | null;
  /** Id corto de `claude --bg` (para attach/stop). */
  runId: string | null;
  cwd: string | null;
  phaseIndex: number | null;
  phaseTotal: number;
  phaseName: string | null;
  tokens: number | null;
  durationMs: number | null;
  startedAt: number | null;
  /** Posición en la cola (1-based) si `kind === "queued"`. */
  queuePos: number | null;
}

/** Espejo de `LAUNCH_GRACE_MS` en src-tauri/src/issue_runs/store.rs. */
export const LAUNCH_GRACE_MS = 90_000;

const RESULT_TONE: Record<string, BadgeTone> = { green: "ok", yellow: "warn", red: "danger" };
const RESULT_LABEL: Record<string, string> = { green: "Green", yellow: "Yellow", red: "Red" };

export const issueRunKey = (ir: IssueRun) => `i:${ir.issueId}:${ir.queuedAt}`;
export const sessionKey = (sessionId: string) => `s:${sessionId}`;

function phaseLabel(detail: RunDetail | null | undefined): string | null {
  if (detail?.currentPhaseIndex == null) return null;
  const total = detail.phases.length || "?";
  const name = detail.currentPhase ? ` · ${detail.currentPhase}` : "";
  return `Phase ${detail.currentPhaseIndex}/${total}${name}`;
}

/** Estado de un run ya visible en `claude agents` (con o sin issue). */
function fromSession(run: RunSummary, detail: RunDetail | null | undefined, title: string) {
  if (run.state === "working") {
    const label = phaseLabel(detail) ?? (detail ? "Running" : "Starting");
    return { kind: "running" as const, label, tone: "accent" as const, title, active: true };
  }
  if (detail?.resultStatus && detail.source !== "live") {
    const r = detail.resultStatus;
    return { kind: "done" as const, label: RESULT_LABEL[r] ?? r, tone: RESULT_TONE[r] ?? "muted", title, active: false };
  }
  if (detail === undefined) return { kind: "done" as const, label: "Finished", tone: "muted" as const, title, active: false };
  if (run.state === "stopped") return { kind: "failed" as const, label: "Stopped", tone: "danger" as const, title, active: false };
  // Terminó sin resumen final del workflow: quedó cortado a mitad de camino.
  return { kind: "failed" as const, label: "Interrupted", tone: "danger" as const, title, active: false };
}

export function issueRunBadge(
  ir: IssueRun,
  run: RunSummary | undefined,
  detail: RunDetail | null | undefined,
): RunBadge & { kind: RunKind } {
  const title = `${ir.workflow} · open run`;
  if (ir.status === "failed") {
    return { kind: "failed", label: "Launch failed", tone: "danger", title: ir.error ?? "Launch failed", active: false };
  }
  if (ir.status === "queued") return { kind: "queued", label: "Queued", tone: "muted", title, active: true };
  if (ir.status === "launching") return { kind: "queued", label: "Launching", tone: "muted", title, active: true };
  // Lanzado pero todavía no aparece en `claude agents`. Pasada la misma gracia que usa
  // el backend se deja relanzar.
  if (!run) {
    const fresh = ir.launchedAt != null && Date.now() - ir.launchedAt < LAUNCH_GRACE_MS;
    return fresh
      ? { kind: "starting", label: "Starting", tone: "accent", title, active: true }
      : {
          kind: "failed",
          label: "Lost",
          tone: "danger",
          title: `Run ${ir.runId ?? ""} no longer shows up in claude agents`,
          active: false,
        };
  }
  return fromSession(run, detail, title);
}

function durationOf(run: RunSummary | undefined, detail: RunDetail | null | undefined): number | null {
  if (detail?.durationMs != null && run?.state !== "working") return detail.durationMs;
  if (run?.startedAt != null) {
    if (run.state === "working") return Date.now() - run.startedAt;
    return detail?.durationMs ?? null;
  }
  return detail?.durationMs ?? null;
}

function base(detail: RunDetail | null | undefined, run: RunSummary | undefined) {
  return {
    phaseIndex: detail?.currentPhaseIndex ?? null,
    phaseTotal: detail?.phases.length ?? 0,
    phaseName: detail?.currentPhase ?? null,
    tokens: detail?.totalTokens ?? null,
    durationMs: durationOf(run, detail),
    startedAt: run?.startedAt ?? null,
  };
}

export function viewOfIssueRun(
  ir: IssueRun,
  run: RunSummary | undefined,
  detail: RunDetail | null | undefined,
  queuePos: number | null,
): RunView {
  const badge = issueRunBadge(ir, run, detail);
  return {
    ...badge,
    label: badge.kind === "queued" && queuePos != null && ir.status === "queued" ? `Queued · #${queuePos}` : badge.label,
    key: issueRunKey(ir),
    ir,
    run,
    detail,
    issueId: ir.issueId,
    identifier: ir.identifier,
    workflow: ir.workflow,
    runId: ir.runId ?? run?.id ?? null,
    cwd: ir.cwd,
    queuePos: badge.kind === "queued" ? queuePos : null,
    ...base(detail, run),
  };
}

export function viewOfSession(run: RunSummary, detail: RunDetail | null | undefined): RunView {
  const s = fromSession(run, detail, "Open run");
  return {
    ...s,
    key: sessionKey(run.sessionId),
    ir: null,
    run,
    detail,
    issueId: null,
    identifier: null,
    workflow: detail?.workflowName ?? null,
    runId: run.id,
    cwd: run.cwd,
    queuePos: null,
    ...base(detail, run),
  };
}
