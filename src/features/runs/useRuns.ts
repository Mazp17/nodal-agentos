import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getRunDetail, listIssueRuns, listRuns } from "./api";
import type { IssueRun, RunDetail, RunSummary } from "./types";

const POLL_MS = 3000;
/** Runs lanzados a mano (sin issue) que se siguen mostrando. */
const MAX_OTHER_RUNS = 6;
/** Polls que se sigue pidiendo el detalle de un run terminado sin resumen final. */
const SETTLE_TRIES = 3;

export type Details = Record<string, RunDetail | null>;

export interface RunsState {
  runs: RunSummary[];
  /** Run vigente por issue (el de queuedAt mayor), más recientes primero. */
  current: IssueRun[];
  byIssue: Map<string, IssueRun>;
  /** Runs recientes que no corresponden a ninguna issue. */
  otherRuns: RunSummary[];
  details: Details;
  error: string | null;
  refresh: () => Promise<void>;
}

export function findRun(runs: RunSummary[], ir: IssueRun): RunSummary | undefined {
  return runs.find((r) => (ir.sessionId && r.sessionId === ir.sessionId) || (ir.runId && r.id === ir.runId));
}

function currentIssueRuns(all: IssueRun[]): IssueRun[] {
  const latest = new Map<string, IssueRun>();
  for (const ir of all) {
    const prev = latest.get(ir.issueId);
    if (!prev || ir.queuedAt > prev.queuedAt) latest.set(ir.issueId, ir);
  }
  return [...latest.values()].sort((a, b) => b.queuedAt - a.queuedAt);
}

function otherRunsOf(runs: RunSummary[], all: IssueRun[]): RunSummary[] {
  return runs.filter((r) => !all.some((ir) => findRun([r], ir))).slice(0, MAX_OTHER_RUNS);
}

/** Polling de `list_runs` + `list_issue_runs` (y detalles) mientras `enabled`. */
export function useRuns(enabled: boolean): RunsState {
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [issueRuns, setIssueRuns] = useState<IssueRun[]>([]);
  const [details, setDetails] = useState<Details>({});
  const [error, setError] = useState<string | null>(null);

  // Sesiones cuyo detalle ya no cambia (no están working): se piden una sola vez.
  const settled = useRef(new Set<string>());
  const liveTries = useRef(new Map<string, number>());
  const inFlight = useRef<Promise<void> | null>(null);
  const again = useRef(false);

  const pollOnce = useCallback(async () => {
    const [r, ir] = await Promise.allSettled([listRuns(), listIssueRuns()]);
    const errors: string[] = [];
    if (r.status === "fulfilled") setRuns(r.value);
    else errors.push(`No se pudieron listar los runs: ${String(r.reason)}`);
    if (ir.status === "fulfilled") setIssueRuns(ir.value);
    else errors.push(`No se pudieron listar los runs de issues: ${String(ir.reason)}`);
    setError(errors.length ? errors.join("\n") : null);
    if (r.status === "rejected") return;

    const all = r.value;
    const irs = ir.status === "fulfilled" ? ir.value : [];
    const linked = currentIssueRuns(irs)
      .map((x) => findRun(all, x))
      .filter((x): x is RunSummary => x !== undefined);
    const relevant = [...linked, ...otherRunsOf(all, irs)];
    const pending = relevant.filter((x) => x.state === "working" || !settled.current.has(x.sessionId));
    const fetched = await Promise.all(
      pending.map(async (x) => {
        try {
          const d = await getRunDetail(x.sessionId, x.cwd ?? "");
          // Al terminar, el resumen final puede tardar un poco en aparecer: no se da por
          // cerrado un detalle `live` hasta varios polls después.
          if (x.state !== "working") {
            const n = (liveTries.current.get(x.sessionId) ?? 0) + 1;
            liveTries.current.set(x.sessionId, n);
            if (d === null || d.source === "final" || n >= SETTLE_TRIES) settled.current.add(x.sessionId);
          }
          return [x.sessionId, d] as const;
        } catch {
          return null; // se reintenta en el próximo poll
        }
      }),
    );
    const updates = fetched.filter((x) => x !== null);
    if (updates.length) setDetails((prev) => ({ ...prev, ...Object.fromEntries(updates) }));
  }, []);

  // Sin solapamiento: si piden un refresh con un poll en vuelo, se repite al terminar.
  const refresh = useCallback((): Promise<void> => {
    if (inFlight.current) {
      again.current = true;
      return inFlight.current;
    }
    const run = (async () => {
      try {
        do {
          again.current = false;
          await pollOnce();
        } while (again.current);
      } finally {
        inFlight.current = null;
      }
    })();
    inFlight.current = run;
    return run;
  }, [pollOnce]);

  useEffect(() => {
    if (!enabled) return;
    let timer: ReturnType<typeof setTimeout>;
    let cancelled = false;
    const loop = async () => {
      await refresh();
      if (!cancelled) timer = setTimeout(loop, POLL_MS);
    };
    loop();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [enabled, refresh]);

  const current = useMemo(() => currentIssueRuns(issueRuns), [issueRuns]);
  const byIssue = useMemo(() => new Map(current.map((ir) => [ir.issueId, ir])), [current]);
  const otherRuns = useMemo(() => otherRunsOf(runs, issueRuns), [runs, issueRuns]);

  return { runs, current, byIssue, otherRuns, details, error, refresh };
}
