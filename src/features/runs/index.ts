// STUB(F2-E): replaced by G (runs). Contrato mínimo que consume E: `RunBadge({ run })`.
import { createElement } from "react";
import type { Run } from "../../domain/types";

type Tone = "accent" | "ok" | "warn" | "danger" | "muted";

function badgeOf(run: Run): { label: string; tone: Tone; live: boolean } {
  switch (run.status) {
    case "queued":
      return { label: "Queued", tone: "muted", live: false };
    case "launching":
      return { label: "Launching", tone: "muted", live: true };
    case "launched":
      return { label: run.kind === "review" ? "Reviewing" : "Running", tone: "accent", live: true };
    case "failed":
      return { label: "Launch failed", tone: "danger", live: false };
    case "canceled":
      return { label: "Canceled", tone: "muted", live: false };
    case "finished":
      if (run.kind === "review" && run.verdict) {
        return run.verdict.pass ? { label: "Review passed", tone: "ok", live: false } : { label: "Review failed", tone: "danger", live: false };
      }
      switch (run.outcome) {
        case "green":
          return { label: run.prUrl ? "PR ready · Green" : "Green", tone: "ok", live: false };
        case "yellow":
          return { label: "Yellow", tone: "warn", live: false };
        case "red":
          return { label: "Red", tone: "danger", live: false };
        case "stopped":
          return { label: "Stopped", tone: "danger", live: false };
        default:
          return { label: "Finished", tone: "muted", live: false };
      }
  }
}

/** STUB(F2-E): replaced by G. */
export function RunBadge({ run }: { run: Run }) {
  const b = badgeOf(run);
  return createElement(
    "span",
    { className: `badge tone-${b.tone}`, title: run.error ?? b.label },
    createElement("span", { className: `dot dot-sm ${b.live ? "pulse" : ""}`, "aria-hidden": true }),
    b.label,
  );
}
