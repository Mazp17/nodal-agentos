import { useEffect, useState } from "react";
import { activitySummary } from "./api";
import type { ActivitySummary } from "./types";

/** Más espaciado que el panel (3 s): es solo el indicador del sidebar. */
const POLL_MS = 10_000;

/**
 * Sesiones trabajando y subagentes activos en los repos mapeados, para el sidebar.
 * Una sola llamada por intervalo para todos los repos; los errores se ignoran (el
 * indicador simplemente no se muestra).
 */
export function useActivitySummary(repoPaths: string[], enabled: boolean): ActivitySummary | null {
  const [summary, setSummary] = useState<ActivitySummary | null>(null);
  const key = repoPaths.join("\n");

  useEffect(() => {
    if (!enabled || !key) {
      setSummary(null);
      return;
    }
    const paths = key.split("\n");
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let lastError = "";
    const loop = async () => {
      try {
        const s = await activitySummary(paths);
        if (!cancelled) setSummary(s);
        lastError = "";
      } catch (e) {
        if (!cancelled) setSummary(null);
        // Sin `claude` fallaría en cada vuelta: se loguea solo cuando cambia.
        if (String(e) !== lastError) console.error("activity_summary", e);
        lastError = String(e);
      }
      if (!cancelled) timer = setTimeout(loop, POLL_MS);
    };
    void loop();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [key, enabled]);

  return summary;
}
