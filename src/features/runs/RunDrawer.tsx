import { useEffect, useRef, useState } from "react";
import { attachRun, cancelQueued, stopRun } from "./api";
import { RunCard } from "./RunCard";
import { LAUNCH_GRACE_MS } from "./status";
import { findRun, type RunsState } from "./useRuns";
import "./runs.css";

interface Props {
  issueId: string;
  runs: RunsState;
  onClose: () => void;
}

export function RunDrawer({ issueId, runs, onClose }: Props) {
  const ir = runs.byIssue.get(issueId);
  const run = ir ? findRun(runs.runs, ir) : undefined;
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const closeRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    closeRef.current?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const act = async (fn: () => Promise<void>, refresh = true) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
      if (refresh) await runs.refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const onStop = (runId: string) => {
    if (!confirm(`¿Detener el run ${runId}?`)) return;
    void act(() => stopRun(runId));
  };

  const queued = ir?.status === "queued" || ir?.status === "launching";

  return (
    <div className="drawer-overlay" onClick={onClose}>
      <aside
        className="drawer"
        role="dialog"
        aria-modal="true"
        aria-labelledby="drawer-title"
        onClick={(e) => e.stopPropagation()}
      >
        <header className="drawer-head">
          <h2 id="drawer-title">
            {ir ? ir.identifier : "Run"}
            {ir && <span className="muted"> · {ir.workflow}</span>}
          </h2>
          <button ref={closeRef} type="button" className="drawer-close" aria-label="Cerrar" onClick={onClose}>
            ×
          </button>
        </header>

        <div className="drawer-body">
          {!ir && <p className="run-empty">No hay runs para esta issue.</p>}

          {ir && queued && (
            <div className="drawer-row">
              <span className="run-empty">
                {ir.status === "launching" ? "Lanzando" : "En cola"} desde{" "}
                {new Date(ir.queuedAt).toLocaleString()}
              </span>
              {ir.status === "queued" && (
                <button type="button" className="btn" disabled={busy} onClick={() => act(async () => { await cancelQueued(ir.issueId); onClose(); })}>
                  Cancelar
                </button>
              )}
            </div>
          )}

          {ir?.status === "failed" && <p className="runs-msg runs-error">{ir.error ?? "Falló el lanzamiento."}</p>}

          {ir?.status === "launched" &&
            (run ? (
              <RunCard run={run} detail={runs.details[run.sessionId]} />
            ) : (
              <p className="run-empty">
                {ir.launchedAt != null && Date.now() - ir.launchedAt < LAUNCH_GRACE_MS
                  ? "Iniciando: esperando que el run aparezca en claude agents…"
                  : "El run no aparece en claude agents; podés relanzarlo desde la card."}
              </p>
            ))}

          {ir?.runId && (
            <div className="drawer-row">
              <button type="button" className="btn" disabled={busy} onClick={() => act(() => attachRun(ir.runId!), false)}>
                Attach
              </button>
              {run?.state === "working" && (
                <button type="button" className="btn danger" disabled={busy} onClick={() => onStop(ir.runId!)}>
                  Stop
                </button>
              )}
            </div>
          )}

          {error && <p className="runs-msg runs-error">{error}</p>}
          {ir && <p className="run-meta drawer-cwd" title={ir.cwd}>{ir.cwd}</p>}
        </div>
      </aside>
    </div>
  );
}
