import { useState, type FormEvent } from "react";
import type { AppConfig } from "../linear/api";
import { cancelQueued, launchRun } from "./api";
import { RunCard } from "./RunCard";
import { issueRunBadge } from "./status";
import type { IssueRun } from "./types";
import { findRun, type RunsState } from "./useRuns";
import "./runs.css";

const OPEN_KEY = "agent-desk.runsPanelOpen";

function readOpen(): boolean {
  try {
    return localStorage.getItem(OPEN_KEY) !== "false";
  } catch {
    return true;
  }
}
function writeOpen(v: boolean) {
  try {
    localStorage.setItem(OPEN_KEY, String(v));
  } catch {
    /* ignorar */
  }
}

interface Props {
  runs: RunsState;
  config: AppConfig;
  onOpenRun: (issueId: string) => void;
}

export function RunsPanel({ runs, config, onOpenRun }: Props) {
  const [open, setOpen] = useState(readOpen);
  const [manual, setManual] = useState(false);
  const [itemError, setItemError] = useState<string | null>(null);

  const toggle = () => {
    setOpen(!open);
    writeOpen(!open);
  };

  const onCancel = async (issueId: string) => {
    setItemError(null);
    try {
      await cancelQueued(issueId);
      await runs.refresh();
    } catch (e) {
      setItemError(String(e));
    }
  };

  return (
    <section className={`runs-panel ${open ? "" : "collapsed"}`}>
      <header className="runs-head">
        <button type="button" className="runs-toggle" aria-expanded={open} onClick={toggle}>
          <span aria-hidden>{open ? "▾" : "▸"}</span> Runs <span className="runs-count">{runs.current.length}</span>
        </button>
        {open && (
          <button type="button" className="btn btn-sm" aria-expanded={manual} onClick={() => setManual(!manual)}>
            Lanzar manual
          </button>
        )}
      </header>

      {open && (
        <>
          {manual && <ManualLaunch config={config} onLaunched={() => void runs.refresh()} />}
          {runs.error && <p className="runs-msg runs-error">{runs.error}</p>}
          {itemError && <p className="runs-msg runs-error">{itemError}</p>}

          <div className="runs-list">
            {runs.current.length === 0 && !runs.error && (
              <p className="run-empty">Todavía no se lanzó ningún run desde el board.</p>
            )}
            {runs.current.map((ir) => (
              <IssueRunItem key={ir.issueId} ir={ir} runs={runs} onOpen={onOpenRun} onCancel={onCancel} />
            ))}
          </div>

          {manual && runs.otherRuns.length > 0 && (
            <>
              <h3 className="runs-subhead">Otros runs recientes</h3>
              <div className="runs-list">
                {runs.otherRuns.map((r) => (
                  <RunCard key={r.sessionId} run={r} detail={runs.details[r.sessionId]} />
                ))}
              </div>
            </>
          )}
        </>
      )}
    </section>
  );
}

function IssueRunItem({
  ir,
  runs,
  onOpen,
  onCancel,
}: {
  ir: IssueRun;
  runs: RunsState;
  onOpen: (issueId: string) => void;
  onCancel: (issueId: string) => void;
}) {
  const run = findRun(runs.runs, ir);
  const detail = run ? runs.details[run.sessionId] : undefined;
  const badge = issueRunBadge(ir, run, detail);

  // El click en cualquier parte abre el drawer; para teclado está el botón del encabezado.
  return (
    <div className="runs-item" onClick={() => onOpen(ir.issueId)}>
      <div className="runs-item-head">
        <button type="button" className="runs-item-open" title="Ver detalle">
          <strong>{ir.identifier}</strong> <span className="muted">· {ir.workflow}</span>
        </button>
        <span className={`run-badge tone-${badge.tone}`} title={badge.title}>
          {badge.label}
        </span>
        {ir.status === "queued" && (
          <button
            type="button"
            className="btn btn-sm"
            onClick={(e) => {
              e.stopPropagation();
              onCancel(ir.issueId);
            }}
          >
            Cancelar
          </button>
        )}
      </div>
      {ir.status === "failed" && <p className="runs-msg runs-error">{ir.error ?? "Falló el lanzamiento."}</p>}
      {ir.status === "launched" && run && <RunCard run={run} detail={detail} />}
    </div>
  );
}

function ManualLaunch({ config, onLaunched }: { config: AppConfig; onLaunched: () => void }) {
  const [cwd, setCwd] = useState(config.repos[0]?.path ?? "");
  const [prompt, setPrompt] = useState("");
  const [launching, setLaunching] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setLaunching(true);
    setMsg(null);
    try {
      const ref = await launchRun(cwd.trim(), prompt);
      setMsg({ ok: true, text: `Lanzado: ${ref.id}` });
      onLaunched();
    } catch (err) {
      setMsg({ ok: false, text: String(err) });
    } finally {
      setLaunching(false);
    }
  };

  return (
    <>
      <form className="runs-form" onSubmit={onSubmit}>
        <input
          type="text"
          aria-label="Carpeta"
          className="runs-input runs-input-cwd"
          value={cwd}
          onChange={(e) => setCwd(e.target.value)}
          placeholder="/ruta/al/repo"
        />
        <input
          type="text"
          aria-label="Prompt"
          className="runs-input"
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="/skill args"
        />
        <button type="submit" className="btn primary" disabled={launching || !cwd.trim() || !prompt.trim()}>
          {launching ? "Lanzando…" : "Lanzar"}
        </button>
      </form>
      {msg && <p className={msg.ok ? "runs-msg" : "runs-msg runs-error"}>{msg.text}</p>}
    </>
  );
}
