import type { AgentInfo, AgentState, RunDetail, RunSummary } from "./types";

const AGENT_ICON: Record<AgentState, string> = {
  done: "✓",
  running: "⟳",
  queued: "⏳",
  failed: "✗",
  unknown: "·",
};

const RUN_STATE_LABEL: Record<string, string> = {
  working: "en curso",
  done: "terminado",
  stopped: "detenido",
};

export function formatTokens(n: number | null | undefined): string {
  if (n == null) return "–";
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${Math.round(n / 1_000)}k`;
  return String(n);
}

export function formatDuration(ms: number | null | undefined): string {
  if (ms == null) return "–";
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** Estado a mostrar: un run que ya no trabaja pero sin resumen final quedó cortado. */
function runStateLabel(run: RunSummary, detail: RunDetail | null | undefined): string {
  if (detail?.source === "live" && run.state && run.state !== "working") return "interrumpido";
  return RUN_STATE_LABEL[run.state ?? ""] ?? run.state ?? "desconocido";
}

function AgentRow({ agent }: { agent: AgentInfo }) {
  const tool = agent.lastToolName
    ? agent.lastToolSummary
      ? `${agent.lastToolName}: ${agent.lastToolSummary}`
      : agent.lastToolName
    : null;
  return (
    <li className={`run-agent run-agent-${agent.state}`}>
      <span className="run-agent-icon" aria-label={agent.state} title={agent.state}>
        {AGENT_ICON[agent.state]}
      </span>
      <span className="run-agent-label" title={agent.label}>
        {agent.label}
      </span>
      <span className="run-agent-model">{agent.model?.replace(/^claude-/, "") ?? ""}</span>
      <span className="run-agent-tokens">{agent.tokens != null ? formatTokens(agent.tokens) : ""}</span>
      {tool && (
        <span className="run-agent-tool" title={tool}>
          {tool}
        </span>
      )}
    </li>
  );
}

interface Props {
  run: RunSummary;
  /** `undefined` = todavía no se pidió; `null` = la sesión no tiene workflow. */
  detail: RunDetail | null | undefined;
}

export function RunCard({ run, detail }: Props) {
  const stateLabel = runStateLabel(run, detail);
  const phaseTotal = detail?.phases.length ?? 0;
  const started = run.startedAt ? new Date(run.startedAt).toLocaleString() : null;

  return (
    <article className="run-card">
      <header className="run-card-head">
        <h3 title={run.sessionId}>{run.name ?? run.id}</h3>
        {detail?.resultStatus && (
          <span className={`run-result run-result-${detail.resultStatus}`} title={`resultado: ${detail.resultStatus}`}>
            {detail.resultStatus}
          </span>
        )}
        <span className={`run-state run-state-${stateLabel.replace(/\s/g, "-")}`}>{stateLabel}</span>
      </header>

      <p className="run-meta">
        <code>{run.id}</code>
        {detail?.workflowName && <> · {detail.workflowName}</>}
        {started && <> · {started}</>}
      </p>

      {detail === undefined && <p className="run-empty">Cargando…</p>}
      {detail === null && <p className="run-empty">Sin workflow en esta sesión.</p>}

      {detail && (
        <>
          {detail.currentPhase && (
            <p className="run-phase">
              Fase {detail.currentPhaseIndex ?? "?"}/{phaseTotal || "?"}: <strong>{detail.currentPhase}</strong>
            </p>
          )}
          {detail.agents.length > 0 && (
            <ul className="run-agents">
              {detail.agents.map((a, i) => (
                <AgentRow key={a.agentId ?? `${a.label}-${i}`} agent={a} />
              ))}
            </ul>
          )}
          <footer className="run-totals">
            <span>{detail.agentCount} agentes</span>
            <span>{formatTokens(detail.totalTokens)} tokens</span>
            {detail.totalToolCalls != null && <span>{detail.totalToolCalls} tools</span>}
            <span>{formatDuration(detail.durationMs)}</span>
            {detail.workflowCount > 1 && <span>+{detail.workflowCount - 1} workflows previos</span>}
          </footer>
        </>
      )}
    </article>
  );
}
