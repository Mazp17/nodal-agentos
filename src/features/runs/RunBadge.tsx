import type { Run } from "../../domain/types";
import { useRuns } from "../../domain/hooks/runs";
import { deriveRunView, type RunView } from "./status";
import "./runs.css";

const EMPTY_CTX = { live: [], details: {}, queuePos: new Map(), reviews: new Map(), now: 0 };

const isView = (x: Run | RunView): x is RunView => "run" in x && "phase" in x;

/** Vista derivada de un run: la del store compartido si ya lo tiene, o una sin datos en vivo. */
export function useRunView(run: Run | RunView): RunView {
  const { byId } = useRuns();
  if (isView(run)) return run;
  return byId.get(run.id) ?? deriveRunView(run, { ...EMPTY_CTX, now: Date.now() });
}

/** Píldora de estado de un run ("Phase 3/9 · Implement", "Needs permission", "PR #12 · Green"). */
export function RunBadge({ run, className }: { run: Run | RunView; className?: string }) {
  const v = useRunView(run);
  return (
    <span className={`badge tone-${v.tone} ${className ?? ""}`} title={v.run.error ?? v.label}>
      <span className={`dot dot-sm ${v.pulse ? "pulse" : ""}`} aria-hidden />
      {v.label}
    </span>
  );
}

/**
 * Barra de fases segmentada (cards del board). Solo para workflows con fases conocidas;
 * para agentes y Claude no dibuja nada.
 */
export function PhaseSegments({ run }: { run: Run | RunView }) {
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
