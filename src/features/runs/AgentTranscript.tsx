import { useEffect, useRef, useState } from "react";
import { formatDuration, formatTokens } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { getRunTranscript } from "../../domain/api";
import { getAgentTranscript } from "./api";
import type { RunView } from "./status";
import type { AgentInfo, AgentState, Transcript, TranscriptItem } from "./types";
import "./run-detail.css";
import "./transcript.css";

const POLL_MS = 3000;
export const DEFAULT_LIMIT = 200;
export const FULL_LIMIT = 1000;
/** Archivos grandes: re-leerlos cada 3 s cuesta; se espacia el polling. */
const BIG_FILE = 4 * 1024 * 1024;
const POLL_BIG_MS = 10_000;

export const AGENT_STATUS: Record<AgentState, { label: string; tone: string }> = {
  done: { label: "Done", tone: "ok" },
  running: { label: "Running", tone: "accent" },
  queued: { label: "Pending", tone: "muted" },
  failed: { label: "Failed", tone: "danger" },
  unknown: { label: "Unknown", tone: "muted" },
};

export const modelName = (m: string | null) => m?.replace(/^claude-/, "") ?? "—";

export type Load = { status: "loading" } | { status: "error"; error: string } | { status: "ok"; transcript: Transcript | null };

/**
 * Lee un transcript con `fetch` (clave `key`; `null` no carga) y lo repite mientras `live`.
 * Un fallo en un poll no borra lo que ya se mostraba.
 */
function usePolledTranscript(key: string | null, fetch: () => Promise<Transcript | null>, live: boolean): Load {
  const [state, setState] = useState<{ key: string; load: Load } | null>(null);
  const fetchRef = useRef(fetch);
  fetchRef.current = fetch;

  useEffect(() => {
    if (key === null) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    let lastBytes = 0;
    let first = true;
    const load = async () => {
      // Ventana oculta: se saltea la consulta y se reintenta en la próxima vuelta.
      if (!first && document.hidden) {
        timer = setTimeout(load, POLL_MS);
        return;
      }
      first = false;
      try {
        const t = await fetchRef.current();
        lastBytes = t?.bytes ?? 0;
        if (!cancelled) setState({ key, load: { status: "ok", transcript: t } });
      } catch (e) {
        if (!cancelled) setState((prev) => (prev?.key === key && prev.load.status === "ok" ? prev : { key, load: { status: "error", error: String(e) } }));
      }
      if (!cancelled && live) timer = setTimeout(load, lastBytes > BIG_FILE ? POLL_BIG_MS : POLL_MS);
    };
    void load();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [key, live]);

  return key !== null && state?.key === key ? state.load : { status: "loading" };
}

/** Transcript de un subagente de workflow; se repite mientras el agente siga corriendo. */
function useTranscript(sessionId: string | null, cwd: string, wfId: string | null, agentId: string | null, live: boolean, limit: number): Load {
  const key = sessionId && wfId && agentId ? `${sessionId}|${cwd}|${wfId}|${agentId}|${limit}` : null;
  return usePolledTranscript(key, () => getAgentTranscript(sessionId ?? "", cwd, wfId ?? "", agentId ?? "", limit), live);
}

/** Transcript de la sesión principal de un run de agente, Claude o revisor. */
export function useRunTranscript(runId: string | null, live: boolean, limit: number): Load {
  return usePolledTranscript(runId ? `run|${runId}|${limit}` : null, () => getRunTranscript(runId ?? "", limit), live);
}

function firstLine(s: string): string {
  const line = s.split("\n").find((l) => l.trim()) ?? "";
  return line.length > 200 ? `${line.slice(0, 200)}…` : line;
}

function ToolCard({ item }: { item: Extract<TranscriptItem, { kind: "toolUse" }> }) {
  const [open, setOpen] = useState(false);
  const r = item.result;
  const expandable = item.input != null || (r != null && r.text.includes("\n"));
  return (
    <div className={`tr-tool ${r?.isError ? "tr-tool-err" : ""}`}>
      <button
        type="button"
        className="tr-tool-head"
        aria-expanded={expandable ? open : undefined}
        disabled={!expandable}
        onClick={() => setOpen(!open)}
      >
        <span className="tr-tool-name">{item.name}</span>
        <span className="tr-tool-arg ellipsis">{item.summary ?? ""}</span>
        {expandable && <span className="tr-tool-chev" aria-hidden>{open ? "▾" : "▸"}</span>}
      </button>
      {open && item.input && <pre className="tr-pre">{item.input}</pre>}
      <div className="tr-tool-res">
        {r == null ? (
          <span className="tr-dim">→ waiting for result…</span>
        ) : open ? (
          <pre className="tr-pre tr-pre-res">
            {r.text || "(empty)"}
            {r.truncated && <span className="tr-dim"> [truncated]</span>}
          </pre>
        ) : (
          <>→ {r.isError ? "Error: " : ""}{firstLine(r.text) || "(empty)"}</>
        )}
      </div>
    </div>
  );
}

function Item({ item }: { item: TranscriptItem }) {
  switch (item.kind) {
    case "text":
      return <div className="tr-msg">{item.text}</div>;
    case "user":
      return (
        <div className="tr-user">
          <span className="tr-user-label">Message to agent</span>
          {item.text}
        </div>
      );
    case "thinking":
      return (
        <details className="tr-thinking">
          <summary>Thinking</summary>
          <div>{item.text}</div>
        </details>
      );
    case "toolUse":
      return <ToolCard item={item} />;
  }
}

/** Conversación de un transcript (mensajes, herramientas, "Show more" y el estado en vivo). */
export function TranscriptConversation({
  t,
  limit,
  onMore,
  live,
  view,
  waiting,
}: {
  t: Transcript;
  limit: number;
  onMore: () => void;
  live: boolean;
  view: RunView;
  waiting: boolean;
}) {
  return (
    <section className="ap-section tr-conv" aria-live={live ? "polite" : undefined}>
      <h3 className="section-label">Transcript</h3>
      {(t.omitted > 0 || t.partial) && (
        <div className="tr-omitted">
          {t.omitted > 0 && `${t.omitted} earlier item${t.omitted === 1 ? "" : "s"} hidden. `}
          {t.partial && "The transcript file is large; only its beginning and end were read. "}
          {t.omitted > 0 && limit < FULL_LIMIT && (
            <button type="button" className="btn btn-xs" onClick={onMore}>
              Show more
            </button>
          )}
        </div>
      )}
      {t.items.length === 0 && <p className="rd-result-note">No messages yet.</p>}
      {t.items.map((it, i) => (
        <Item key={`${t.omitted + i}`} item={it} />
      ))}
      {live && !waiting && (
        <div className="tr-next">
          <span className="dot dot-sm pulse tone-accent" aria-hidden />
          Waiting for the next event…
        </div>
      )}
      {waiting && (
        <div className="tr-wait">
          {view.waitingFor === "permission prompt"
            ? "Waiting on a permission prompt. Attach to the session to answer it."
            : "Waiting for your input. Attach to respond."}
        </div>
      )}
    </section>
  );
}

/**
 * Pantalla "Subagent transcript" (drawer): prompt, conversación y salida final de un
 * subagente de workflow. Se repite cada 3 s mientras el agente sigue corriendo.
 */
export function AgentTranscript({
  agent,
  phaseNum,
  view,
  workflowId,
  taskRef,
  onClose,
}: {
  agent: AgentInfo;
  phaseNum: number | null;
  view: RunView;
  /** `RunDetail.workflowId` (`wf_...`). */
  workflowId: string | null;
  /** Id visible de la tarea (`PAY-12`), para el subtítulo. */
  taskRef?: string | null;
  onClose: () => void;
}) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const [limit, setLimit] = useState(DEFAULT_LIMIT);
  const st = AGENT_STATUS[agent.state];
  const sessionLive = view.phase === "running" || view.phase === "waiting";
  const live = agent.state === "running" && sessionLive;
  const load = useTranscript(view.run.sessionId, view.run.cwd, workflowId, agent.agentId, live, limit);
  const t = load.status === "ok" ? load.transcript : null;

  const sub = [
    modelName(agent.model ?? t?.model ?? null),
    agent.phase ? `Phase${phaseNum ? ` ${phaseNum}` : ""} ${agent.phase}` : null,
    taskRef,
    view.run.claudeRunId,
  ]
    .filter(Boolean)
    .join(" · ");
  const waiting = agent.state === "running" && view.phase === "waiting";
  const output =
    // Mientras corre, el "último texto" es charla intermedia, no la salida final.
    (agent.state === "running" ? null : t?.finalOutput) ??
    agent.resultPreview ??
    (agent.state === "running" ? "No final output yet." : agent.state === "queued" ? "Not started." : "No output recorded.");
  const outTone = agent.state === "done" ? "ok" : agent.state === "failed" ? "danger" : "none";

  return (
    <>
      <div className="scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="sheet agent-panel" role="dialog" aria-modal="true" aria-labelledby="agent-title" tabIndex={-1}>
        <header className="ap-head">
          <div className="ap-head-main">
            <div className="ap-title-row">
              <h2 id="agent-title" className="ap-title">
                {agent.label}
              </h2>
              <span className={`rd-agent-st tone-${st.tone}`}>
                <span className={`dot dot-sm ${agent.state === "running" ? "pulse" : ""}`} aria-hidden />
                {st.label}
              </span>
              {live && (
                <span className="tr-live">
                  <span className="dot dot-sm pulse" aria-hidden />
                  Live
                </span>
              )}
            </div>
            <span className="ap-sub">{sub}</span>
          </div>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>
        <div className="ap-body">
          <dl className="rd-stats">
            <div>
              <dt>Tokens</dt>
              <dd className="num">{formatTokens(agent.tokens)}</dd>
            </div>
            <div>
              <dt>Tool calls</dt>
              <dd className="num">{agent.toolCalls ?? "—"}</dd>
            </div>
            <div>
              <dt>Time</dt>
              <dd className="num">{formatDuration(agent.durationMs)}</dd>
            </div>
          </dl>

          {!agent.agentId || !workflowId || !view.run.sessionId ? (
            <p className="rd-result-note">No transcript available for this agent.</p>
          ) : load.status === "loading" ? (
            <p className="rd-result-note">Loading transcript…</p>
          ) : load.status === "error" ? (
            <p className="rd-result-note tr-error" role="alert">
              Couldn't load the transcript: {load.error}
            </p>
          ) : t === null ? (
            <p className="rd-result-note">
              {agent.state === "queued" ? "The agent hasn't started yet." : "No transcript file for this agent yet."}
            </p>
          ) : (
            <>
              <section className="ap-section">
                <h3 className="section-label">Input prompt</h3>
                <div className="tr-prompt">{t.prompt ?? "—"}</div>
              </section>
              <TranscriptConversation t={t} limit={limit} onMore={() => setLimit(FULL_LIMIT)} live={live} view={view} waiting={waiting} />
            </>
          )}

          <section className="ap-section">
            <h3 className="section-label">Final output</h3>
            <div className={`ap-out ap-out-${outTone}`}>{output}</div>
          </section>
        </div>
      </div>
    </>
  );
}
