import type { RunLight } from "../../domain/types";
import { useRuns } from "../../domain/hooks/runs";
import { deriveRunView, type RunView } from "./status";
import "./runs.css";

const EMPTY_CTX = { live: [], details: {}, queuePos: new Map(), reviews: new Map(), now: 0 };

const isView = (x: RunLight | RunView): x is RunView => "run" in x && "phase" in x;

/** Derived view of a run: the shared store's if it has one, or one without live data. */
export function useRunView(run: RunLight | RunView): RunView {
  const { byId } = useRuns();
  if (isView(run)) return run;
  return byId.get(run.id) ?? deriveRunView(run, { ...EMPTY_CTX, now: Date.now() });
}

/** Status pill for a run ("Phase 3/9 · Implement", "Needs permission", "PR #12 · Green"). */
export function RunBadge({ run, className }: { run: RunLight | RunView; className?: string }) {
  const v = useRunView(run);
  return (
    <span className={`badge tone-${v.tone} ${className ?? ""}`} title={v.run.error ?? v.label}>
      <span className={`dot dot-sm ${v.pulse ? "pulse" : ""}`} aria-hidden />
      {v.label}
    </span>
  );
}

/**
 * Segmented phase bar (board cards). Only for workflows with known phases;
 * draws nothing for agents and Claude.
 */
export function PhaseSegments({ run }: { run: RunLight | RunView }) {
  const v = useRunView(run);
  const n = v.phaseTotal;
  if (!n) return null;
  const done = v.phase === "finished" ? n : (v.phaseIndex ?? 0);
  return (
    <div
      className={`phase-segs tone-${v.tone}`}
      style={{ gridTemplateColumns: `repeat(${n}, 1fr)` }}
      role="img"
      aria-label={`Phase ${Math.min(done, n)} of ${n}`}
    >
      {Array.from({ length: n }, (_, k) => (
        <span key={k} className={k < done ? (k === done - 1 && v.pulse ? "on cur" : "on") : ""} />
      ))}
    </div>
  );
}
