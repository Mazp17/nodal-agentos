import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { getRunDetail, launchRun, listRuns } from "./api";
import { RunCard } from "./RunCard";
import type { RunDetail, RunSummary } from "./types";
import "./runs.css";

const POLL_MS = 3000;
/** Solo los más recientes: `claude agents --all` trae el historial completo. */
const MAX_RUNS = 12;
/** Polls que se sigue pidiendo el detalle de un run terminado sin resumen final. */
const SETTLE_TRIES = 3;
const DEFAULT_CWD = "/Users/me/Code/nodal-sandbox";
const DEFAULT_PROMPT = "/demo-board volcanes";

type Details = Record<string, RunDetail | null>;

export function RunsPanel() {
  const [runs, setRuns] = useState<RunSummary[]>([]);
  const [details, setDetails] = useState<Details>({});
  const [error, setError] = useState<string | null>(null);
  const [cwd, setCwd] = useState(DEFAULT_CWD);
  const [prompt, setPrompt] = useState(DEFAULT_PROMPT);
  const [launching, setLaunching] = useState(false);
  const [launchMsg, setLaunchMsg] = useState<{ ok: boolean; text: string } | null>(null);

  // Sesiones cuyo detalle ya no cambia (no están working): se piden una sola vez.
  const settled = useRef(new Set<string>());
  const liveTries = useRef(new Map<string, number>());
  const inFlight = useRef(false);

  const poll = useCallback(async () => {
    if (inFlight.current) return;
    inFlight.current = true;
    try {
      const all = (await listRuns()).slice(0, MAX_RUNS);
      setRuns(all);
      setError(null);
      const pending = all.filter((r) => r.state === "working" || !settled.current.has(r.sessionId));
      const fetched = await Promise.all(
        pending.map(async (r) => {
          try {
            const d = await getRunDetail(r.sessionId, r.cwd ?? "");
            // Al terminar, el resumen final puede tardar un poco en aparecer: no se da por
            // cerrado un detalle `live` hasta varios polls después.
            if (r.state !== "working") {
              const n = (liveTries.current.get(r.sessionId) ?? 0) + 1;
              liveTries.current.set(r.sessionId, n);
              if (d === null || d.source === "final" || n >= SETTLE_TRIES) settled.current.add(r.sessionId);
            }
            return [r.sessionId, d] as const;
          } catch {
            return null; // se reintenta en el próximo poll
          }
        }),
      );
      const updates = fetched.filter((x) => x !== null);
      if (updates.length) setDetails((prev) => ({ ...prev, ...Object.fromEntries(updates) }));
    } catch (err) {
      setError(String(err));
    } finally {
      inFlight.current = false;
    }
  }, []);

  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    let cancelled = false;
    const loop = async () => {
      await poll();
      if (!cancelled) timer = setTimeout(loop, POLL_MS);
    };
    loop();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [poll]);

  const onLaunch = async (e: FormEvent) => {
    e.preventDefault();
    setLaunching(true);
    setLaunchMsg(null);
    try {
      const ref = await launchRun(cwd.trim(), prompt);
      setLaunchMsg({ ok: true, text: `Lanzado: ${ref.id}` });
      void poll();
    } catch (err) {
      setLaunchMsg({ ok: false, text: String(err) });
    } finally {
      setLaunching(false);
    }
  };

  return (
    <section className="runs-panel">
      <header className="runs-head">
        <h2>
          Runs en background <span className="runs-count">{runs.length}</span>
        </h2>
        <form className="runs-form" onSubmit={onLaunch}>
          <input
            aria-label="Carpeta"
            className="runs-input runs-input-cwd"
            value={cwd}
            onChange={(e) => setCwd(e.target.value)}
            placeholder="/ruta/al/repo"
          />
          <input
            aria-label="Prompt"
            className="runs-input"
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            placeholder="/skill args"
          />
          <button type="submit" disabled={launching || !cwd.trim() || !prompt.trim()}>
            {launching ? "Lanzando…" : "Lanzar"}
          </button>
        </form>
      </header>

      {launchMsg && <p className={launchMsg.ok ? "runs-msg" : "runs-msg runs-error"}>{launchMsg.text}</p>}
      {error && <p className="runs-msg runs-error">{error}</p>}

      <div className="runs-list">
        {runs.length === 0 && !error && <p className="run-empty">No hay runs en background.</p>}
        {runs.map((r) => (
          <RunCard key={r.sessionId} run={r} detail={details[r.sessionId]} />
        ))}
      </div>
    </section>
  );
}
