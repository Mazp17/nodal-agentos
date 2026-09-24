import { useMemo } from "react";
import { useRuns } from "../../domain/hooks/runs";
import type { RunLight } from "../../domain/types";

export interface PhaseProgress {
  /** 1-based. */
  index: number;
  total: number;
  name: string | null;
}

/**
 * Fase actual de los runs de workflow en marcha (para los segmentos de la card). Sale del
 * store compartido de runs, que ya pide `get_run_detail` de los workflows activos.
 */
export function useRunPhases(runs: RunLight[]): Map<string, PhaseProgress> {
  const { byId } = useRuns();
  return useMemo(() => {
    const out = new Map<string, PhaseProgress>();
    for (const r of runs) {
      if (r.status !== "launched" || r.executor.kind !== "workflow") continue;
      const v = byId.get(r.id);
      if (v && v.phaseIndex != null && v.phaseTotal > 0) {
        out.set(r.id, { index: v.phaseIndex, total: v.phaseTotal, name: v.phaseName });
      }
    }
    return out;
  }, [runs, byId]);
}
