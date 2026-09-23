import type { IssueRun, RunDetail, RunSummary } from "./types";

export type BadgeTone = "accent" | "ok" | "warn" | "danger" | "muted";

export interface RunBadge {
  label: string;
  tone: BadgeTone;
  title: string;
  /** Hay un run en marcha o en cola: no se puede lanzar otro para la issue. */
  active: boolean;
}

/** Espejo de `LAUNCH_GRACE_MS` en src-tauri/src/issue_runs/store.rs. */
export const LAUNCH_GRACE_MS = 90_000;

const RESULT_TONE: Record<string, BadgeTone> = { green: "ok", yellow: "warn", red: "danger" };

export function issueRunBadge(
  ir: IssueRun,
  run: RunSummary | undefined,
  detail: RunDetail | null | undefined,
): RunBadge {
  const title = `${ir.workflow} · ver detalle`;
  if (ir.status === "failed") {
    return { label: "error", tone: "danger", title: ir.error ?? "Falló el lanzamiento", active: false };
  }
  if (ir.status === "queued" || ir.status === "launching") {
    return { label: "en cola", tone: "muted", title, active: true };
  }
  // Lanzado pero todavía no aparece en `claude agents`. Pasada la misma gracia que usa
  // el backend (LAUNCH_GRACE_MS en store.rs) se deja relanzar.
  if (!run) {
    const fresh = ir.launchedAt != null && Date.now() - ir.launchedAt < LAUNCH_GRACE_MS;
    return fresh
      ? { label: "iniciando", tone: "accent", title, active: true }
      : { label: "perdido", tone: "danger", title: `El run ${ir.runId ?? ""} no aparece en claude agents`, active: false };
  }
  if (run.state === "working") {
    if (detail?.currentPhaseIndex != null) {
      const total = detail.phases.length || "?";
      return { label: `fase ${detail.currentPhaseIndex}/${total}`, tone: "accent", title, active: true };
    }
    return { label: detail ? "en curso" : "iniciando", tone: "accent", title, active: true };
  }
  if (detail?.resultStatus && detail.source !== "live") {
    const tone = RESULT_TONE[detail.resultStatus] ?? "muted";
    return { label: detail.resultStatus, tone, title, active: false };
  }
  return { label: "interrumpido", tone: "danger", title, active: false };
}
