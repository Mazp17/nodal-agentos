import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getRunDetail, listIssueRuns, listRuns } from "./api";
import { issueRunKey, viewOfIssueRun, viewOfSession, type RunView } from "./status";
import type { IssueRun, RunDetail, RunSummary } from "./types";

const POLL_MS = 3000;
/** Runs lanzados a mano (sin issue) que se siguen mostrando. */
const MAX_OTHER_RUNS = 6;
/** Polls que se sigue pidiendo el detalle de un run terminado sin resumen final. */
const SETTLE_TRIES = 3;
/** Runs del historial (no vigentes) cuyo detalle también se pide, una vez cada uno. */
const MAX_HISTORY_DETAILS = 40;

export type Details = Record<string, RunDetail | null>;

export interface RunsState {
  runs: RunSummary[];
  /** Run vigente por issue (el de queuedAt mayor), más recientes primero. */
  current: IssueRun[];
  byIssue: Map<string, IssueRun>;
  /** Runs recientes que no corresponden a ninguna issue. */
  otherRuns: RunSummary[];
  details: Details;
  /** Todo el historial de runs de issues (más recientes primero) más los runs sueltos. */
  views: RunView[];
  /** Vista del run vigente por issue. */
  currentView: Map<string, RunView>;
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
    else errors.push(`Couldn't list runs: ${String(r.reason)}`);
    if (ir.status === "fulfilled") setIssueRuns(ir.value);
    else errors.push(`Couldn't list issue runs: ${String(ir.reason)}`);
    setError(errors.length ? errors.join("\n") : null);
    if (r.status === "rejected") return;

    const all = r.value;
    const irs = ir.status === "fulfilled" ? ir.value : [];
    const linked = currentIssueRuns(irs)
      .map((x) => findRun(all, x))
      .filter((x): x is RunSummary => x !== undefined);
    // Historial: se pide una vez (quedan `settled`) para pintar resultado, duración y tokens.
    const history = [...irs]
      .sort((a, b) => b.queuedAt - a.queuedAt)
      .slice(0, MAX_HISTORY_DETAILS)
      .map((x) => findRun(all, x))
      .filter((x): x is RunSummary => x !== undefined && !linked.includes(x));
    const relevant = [...new Set([...linked, ...history, ...otherRunsOf(all, irs)])];
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

  const { views, currentView } = useMemo(() => {
    // Posición en la cola: el más viejo sale primero.
    const queued = issueRuns.filter((x) => x.status === "queued").sort((a, b) => a.queuedAt - b.queuedAt);
    const viewOf = (ir: IssueRun) => {
      const run = findRun(runs, ir);
      const pos = queued.indexOf(ir);
      const isCurrent = current.some((c) => c.issueId === ir.issueId && c.queuedAt === ir.queuedAt);
      return viewOfIssueRun(ir, run, run ? details[run.sessionId] : undefined, pos >= 0 ? pos + 1 : null, isCurrent);
    };
    const views = [
      ...[...issueRuns].sort((a, b) => b.queuedAt - a.queuedAt).map(viewOf),
      ...otherRuns.map((r) => viewOfSession(r, details[r.sessionId])),
    ];
    const byKey = new Map(views.map((v) => [v.key, v]));
    const currentView = new Map<string, RunView>();
    for (const ir of current) {
      const v = byKey.get(issueRunKey(ir));
      if (v) currentView.set(ir.issueId, v);
    }
    return { views, currentView };
  }, [issueRuns, runs, details, otherRuns, current]);

  return { runs, current, byIssue, otherRuns, details, views, currentView, error, refresh };
}
