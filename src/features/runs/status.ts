import type { IssueRun, RunDetail, RunSummary } from "./types";

export type BadgeTone = "accent" | "ok" | "warn" | "danger" | "muted";

export interface RunBadge {
  label: string;
  tone: BadgeTone;
  title: string;
  /** Hay un run en marcha o en cola: no se puede lanzar otro para la issue. */
  active: boolean;
}

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
  // Lanzado pero todavía no aparece en `claude agents`.
  if (!run) return { label: "iniciando", tone: "accent", title, active: true };
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
