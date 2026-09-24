import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Run } from "../../domain/types";

export interface PhaseProgress {
  /** 1-based. */
  index: number;
  total: number;
  name: string | null;
}

/** Lo único que el board lee de `get_run_detail` (el resto lo usa RunDetail). */
interface DetailLite {
  phases: unknown[];
  currentPhase: string | null;
  currentPhaseIndex: number | null;
}

const INTERVAL = 5000;

/**
 * Fase actual de los runs de workflow en marcha (para los segmentos de la card). Solo los
 * workflows tienen fases; agentes y Claude no. Pausado con la ventana oculta.
 */
export function useRunPhases(runs: Run[]): Map<string, PhaseProgress> {
  const targets = useMemo(
    () =>
      runs.filter((r) => r.status === "launched" && r.executor.kind === "workflow" && r.sessionId),
    [runs],
  );
  const sig = targets.map((r) => `${r.id}:${r.sessionId}`).join(",");
  const [phases, setPhases] = useState<Map<string, PhaseProgress>>(new Map());

  useEffect(() => {
    if (!targets.length) {
      setPhases((p) => (p.size ? new Map() : p));
      return;
    }
    let alive = true;
    const load = async () => {
      if (document.hidden) return;
      const out = new Map<string, PhaseProgress>();
      await Promise.all(
        targets.map(async (r) => {
          try {
            const d = await invoke<DetailLite | null>("get_run_detail", { sessionId: r.sessionId, cwd: r.cwd });
            if (d && d.currentPhaseIndex != null && d.phases.length > 0) {
              out.set(r.id, { index: d.currentPhaseIndex, total: d.phases.length, name: d.currentPhase });
            }
          } catch {
            /* sin detalle todavía: la card muestra solo el badge */
          }
        }),
      );
      if (alive) setPhases(out);
    };
    void load();
    const t = window.setInterval(load, INTERVAL);
    return () => {
      alive = false;
      window.clearInterval(t);
    };
    // `sig` resume `targets`: no se reinicia el timer en cada polling de runs.
  }, [sig]);

  return phases;
}
