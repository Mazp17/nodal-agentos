import { useEffect, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import type { Issue } from "../linear/api";
import { formatDateTime, formatDuration, formatTokens } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { RunBadge } from "../linear/Board";
import type { RunActions } from "./actions";
import { getRunDetail } from "./api";
import type { RunView } from "./status";
import type { AgentInfo, AgentState, RunDetail } from "./types";
import "./run-detail.css";

const DETAIL_POLL_MS = 3000;

/**
 * Detalle del run: `useRuns` ya lo trae para el run vigente de cada issue; para runs
 * del historial se pide acá (y se repite mientras siga trabajando).
 */
function useRunDetail(view: RunView): RunDetail | null | undefined {
  const [own, setOwn] = useState<{ sid: string; detail: RunDetail | null } | null>(null);
  const run = view.run;
  const need = view.detail === undefined && run !== undefined;
  const sid = run?.sessionId;
  const cwd = run?.cwd ?? view.cwd ?? "";
  const working = run?.state === "working";

  useEffect(() => {
    if (!need || !sid) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const load = async () => {
      try {
        const d = await getRunDetail(sid, cwd);
        if (!cancelled) setOwn({ sid, detail: d });
      } catch (e) {
        console.error("get_run_detail", e);
      }
      if (!cancelled && working) timer = setTimeout(load, DETAIL_POLL_MS);
    };
    void load();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [need, sid, cwd, working]);

  if (view.detail !== undefined) return view.detail;
  return own && own.sid === sid ? own.detail : undefined;
}

const AGENT_STATUS: Record<AgentState, { label: string; tone: string }> = {
  done: { label: "Done", tone: "ok" },
  running: { label: "Running", tone: "accent" },
  queued: { label: "Pending", tone: "muted" },
  failed: { label: "Failed", tone: "danger" },
  unknown: { label: "Unknown", tone: "muted" },
};

const modelName = (m: string | null) => m?.replace(/^claude-/, "") ?? "—";

function lastAction(a: AgentInfo): string {
  if (!a.lastToolName) return "—";
  return a.lastToolSummary ? `${a.lastToolName} ${a.lastToolSummary}` : a.lastToolName;
}

interface Props {
  view: RunView;
  issue: Issue | undefined;
  backLabel: string;
  actions: RunActions;
  onBack: () => void;
  onOpenIssue: (issueId: string) => void;
  onRunAgain?: () => void;
}

export function RunDetailView({ view, issue, backLabel, actions, onBack, onOpenIssue, onRunAgain }: Props) {
  const detail = useRunDetail(view);
  const [agentIdx, setAgentIdx] = useState<number | null>(null);
  const working = view.run?.state === "working";
  const title = issue?.title ?? view.run?.name ?? (view.identifier ? view.identifier : "Manual run");
  const phases = detail?.phases ?? [];
  const cur = detail?.currentPhaseIndex ?? null;
  const finished = view.kind === "done";
  const stoppedEarly = view.kind === "failed";

  // Agrupa subagentes por fase (en el orden del workflow); los que no tienen fase van al final.
  const groups: { title: string; num: number | null; agents: [AgentInfo, number][] }[] = phases.map((ph, i) => ({
    title: ph.title,
    num: i + 1,
    agents: [],
  }));
  const extra: [AgentInfo, number][] = [];
  detail?.agents.forEach((a, i) => {
    const g = groups.find((x) => x.title === a.phase);
    (g ? g.agents : extra).push([a, i]);
  });
  if (extra.length) groups.push({ title: phases.length ? "Other" : "Agents", num: null, agents: extra });
  const shownGroups = groups.filter((g) => g.agents.length > 0 || g.num !== null);

  const agent = agentIdx != null ? detail?.agents[agentIdx] : undefined;

  return (
    <div className="rd">
      <div className="rd-head">
        <div className="rd-head-main">
          <button type="button" className="rd-back" onClick={onBack}>
            ← {backLabel}
          </button>
          <div className="rd-ids">
            {view.identifier && <span className="rd-issue">{view.identifier}</span>}
            <RunBadge view={view} />
            {view.runId && <span className="rd-runid">{view.runId}</span>}
          </div>
          <h1 className="rd-title">{title}</h1>
          <div className="rd-meta">
            {view.cwd && (
              <span>
                Repo <span className="rd-meta-v mono">{view.cwd}</span>
              </span>
            )}
            {(view.workflow ?? detail?.workflowName) && (
              <span>
                Workflow <span className="rd-meta-v">{view.workflow ?? detail?.workflowName}</span>
              </span>
            )}
            {view.startedAt != null && (
              <span>
                Started <span className="rd-meta-v">{formatDateTime(view.startedAt)}</span>
              </span>
            )}
            <span>
              Elapsed <span className="rd-meta-v num">{view.kind === "queued" ? "—" : formatDuration(detail?.durationMs ?? view.durationMs)}</span>
            </span>
            <span>
              Tokens <span className="rd-meta-v num">{formatTokens(detail?.totalTokens ?? view.tokens)}</span>
            </span>
            {detail?.totalToolCalls != null && (
              <span>
                Tool calls <span className="rd-meta-v num">{detail.totalToolCalls}</span>
              </span>
            )}
          </div>
        </div>
        <div className="rd-actions">
          {working && view.runId && (
            <button type="button" className="btn btn-danger" disabled={actions.busy} onClick={() => void actions.stop(view)}>
              Stop
            </button>
          )}
          {view.runId && view.kind !== "queued" && (
            <button type="button" className="btn" disabled={actions.busy} onClick={() => void actions.attach(view)}>
              Attach
            </button>
          )}
          {view.ir?.status === "queued" && (
            <button
              type="button"
              className="btn btn-danger"
              disabled={actions.busy}
              onClick={async () => {
                if (await actions.cancel(view)) onBack();
              }}
            >
              Remove from queue
            </button>
          )}
          {view.cwd && (
            <button
              type="button"
              className="btn"
              onClick={() => revealItemInDir(view.cwd!).catch((e) => console.error("reveal", e))}
            >
              Show in Finder
            </button>
          )}
          {view.issueId && issue && (
            <button type="button" className="btn" onClick={() => onOpenIssue(view.issueId!)}>
              Open issue
            </button>
          )}
          {stoppedEarly && onRunAgain && (
            <button type="button" className="btn btn-primary" onClick={onRunAgain}>
              Run again
            </button>
          )}
        </div>
      </div>

      {view.ir?.status === "failed" && (
        <div className="rd-alert rd-alert-red" role="alert">
          <span className="dot dot-lg" aria-hidden />
          <div className="rd-alert-body">
            <span className="rd-alert-title">Launch failed</span>
            <span className="rd-alert-text">{view.ir.error ?? "The run couldn't be started."}</span>
          </div>
        </div>
      )}
      {view.kind === "failed" && view.ir?.status === "launched" && !view.run && (
        <div className="rd-alert rd-alert-red" role="alert">
          <span className="dot dot-lg" aria-hidden />
          <div className="rd-alert-body">
            <span className="rd-alert-title">Run lost</span>
            <span className="rd-alert-text">
              It was launched but never showed up in claude agents. You can launch the issue again.
            </span>
          </div>
        </div>
      )}

      {phases.length > 0 && (
        <div className="panel rd-timeline">
          <ol className="tl" style={{ gridTemplateColumns: `repeat(${phases.length}, minmax(0, 1fr))` }}>
            {phases.map((ph, i) => {
              const num = i + 1;
              const done = finished || (cur != null && num < cur);
              const isCur = !finished && cur === num;
              const state = done ? "done" : isCur ? (stoppedEarly ? "failed" : "current") : "pending";
              return (
                <li key={ph.title + i} className={`tl-step tl-${state} tone-${view.tone}`} aria-current={isCur ? "step" : undefined}>
                  <div className="tl-mark-row">
                    <span className="tl-mark">{done ? "✓" : state === "failed" ? "✕" : num}</span>
                    {i < phases.length - 1 && <span className="tl-line" />}
                  </div>
                  <span className="tl-name" title={ph.detail ?? ph.title}>
                    {ph.title}
                  </span>
                </li>
              );
            })}
          </ol>
        </div>
      )}

      <div className="rd-grid">
        <div className="panel rd-agents">
          <div className="table-head rd-cols">
            <span>Subagent</span>
            <span>Model</span>
            <span>Status</span>
            <span className="text-right">Tokens</span>
            <span className="text-right">Time</span>
            <span>Last action</span>
          </div>
          {detail === undefined && view.run && <div className="rd-agents-empty">Loading agents…</div>}
          {detail === null && <div className="rd-agents-empty">This session didn't launch a workflow.</div>}
          {!view.run && detail === undefined && (
            <div className="rd-agents-empty">
              {view.kind === "queued" ? "Agents appear once the run starts." : "No agent data for this run."}
            </div>
          )}
          {detail && detail.agents.length === 0 && <div className="rd-agents-empty">No subagents yet.</div>}
          {detail &&
            detail.agents.length > 0 &&
            shownGroups.map((g) => {
              const isCur = !finished && g.num != null && g.num === cur;
              const past = finished || (g.num != null && cur != null && g.num < cur);
              return (
                <div key={g.title + (g.num ?? "x")} className="rd-group">
                  <div className={`rd-group-head ${isCur ? "current" : past ? "past" : ""}`}>
                    {g.num != null && <span className="rd-group-num">{String(g.num).padStart(2, "0")}</span>}
                    <span className="rd-group-name">{g.title}</span>
                    {isCur && <span className="rd-group-note">current</span>}
                  </div>
                  {g.agents.map(([a, idx]) => {
                    const st = AGENT_STATUS[a.state];
                    return (
                      <button
                        key={a.agentId ?? `${a.label}-${idx}`}
                        type="button"
                        className="rd-cols rd-agent"
                        onClick={() => setAgentIdx(idx)}
                      >
                        <span className="rd-agent-label ellipsis">{a.label}</span>
                        <span className="rd-agent-model ellipsis">{modelName(a.model)}</span>
                        <span className={`rd-agent-st tone-${st.tone}`}>
                          <span className={`dot dot-sm ${a.state === "running" ? "pulse" : ""}`} aria-hidden />
                          {st.label}
                        </span>
                        <span className="text-right num rd-agent-num">{formatTokens(a.tokens)}</span>
                        <span className="text-right num rd-agent-num">{formatDuration(a.durationMs)}</span>
                        <span className="rd-agent-last ellipsis">{lastAction(a)}</span>
                      </button>
                    );
                  })}
                </div>
              );
            })}
        </div>

        <ResultCard view={view} detail={detail} />
      </div>

      {agent && (
        <AgentPanel
          agent={agent}
          phaseNum={phases.findIndex((p) => p.title === agent.phase) + 1 || null}
          view={view}
          onClose={() => setAgentIdx(null)}
        />
      )}
    </div>
  );
}

const RESULT_TEXT: Record<string, string> = {
  green: "Result: green",
  yellow: "Result: yellow",
  red: "Result: red",
};

function ResultCard({ view, detail }: { view: RunView; detail: RunDetail | null | undefined }) {
  let body;
  if (view.kind === "queued") {
    body = (
      <p className="rd-result-note">
        {view.queuePos != null ? `Waiting in queue at position #${view.queuePos}.` : "Waiting for a free slot."}
      </p>
    );
  } else if (view.kind === "running" || view.kind === "starting") {
    body = (
      <p className="rd-result-note">
        The result appears when the run finishes.{view.phaseName ? ` Currently in ${view.phaseName}.` : ""}
      </p>
    );
  } else if (detail?.resultStatus && detail.source === "final") {
    body = (
      <>
        <div className={`rd-result-status tone-${view.tone}`}>
          <span className="dot dot-lg" aria-hidden />
          {RESULT_TEXT[detail.resultStatus] ?? detail.resultStatus}
        </div>
        {detail.status && <p className="rd-result-note">Workflow status: {detail.status}</p>}
      </>
    );
  } else {
    body = (
      <>
        <div className={`rd-result-status tone-${view.tone}`}>
          <span className="dot dot-lg" aria-hidden />
          {view.label}
        </div>
        <p className="rd-result-note">
          {view.ir?.status === "failed"
            ? "The run never started."
            : detail === null
              ? "The session ended without running a workflow."
              : "The session ended without a final workflow summary."}
        </p>
      </>
    );
  }

  return (
    <div className="panel rd-result">
      <h2 className="rd-result-label">Result</h2>
      {body}
      {detail && (
        <dl className="rd-stats">
          <div>
            <dt>Agents</dt>
            <dd className="num">{detail.agentCount}</dd>
          </div>
          <div>
            <dt>Tool calls</dt>
            <dd className="num">{detail.totalToolCalls ?? "—"}</dd>
          </div>
          <div>
            <dt>Tokens</dt>
            <dd className="num">{formatTokens(detail.totalTokens)}</dd>
          </div>
        </dl>
      )}
      {detail && detail.workflowCount > 1 && (
        <p className="rd-result-note">
          This session ran {detail.workflowCount} workflows; showing the latest.
        </p>
      )}
    </div>
  );
}

/** Subagente: lo que expone `get_run_detail` (sin prompt ni conversación completa). */
function AgentPanel({
  agent,
  phaseNum,
  view,
  onClose,
}: {
  agent: AgentInfo;
  phaseNum: number | null;
  view: RunView;
  onClose: () => void;
}) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const st = AGENT_STATUS[agent.state];
  const sub = [
    modelName(agent.model),
    agent.phase ? `Phase${phaseNum ? ` ${phaseNum}` : ""} ${agent.phase}` : null,
    view.identifier,
    view.runId,
  ]
    .filter(Boolean)
    .join(" · ");
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
          <section className="ap-section">
            <h3 className="section-label">Last action</h3>
            {agent.lastToolName ? (
              <div className="ap-tool">
                <span className="ap-tool-name">{agent.lastToolName}</span>
                {agent.lastToolSummary && <span className="ap-tool-arg">{agent.lastToolSummary}</span>}
              </div>
            ) : (
              <p className="rd-result-note">No tool calls recorded.</p>
            )}
          </section>
          <section className="ap-section">
            <h3 className="section-label">Final output</h3>
            <div className={`ap-out ap-out-${outTone}`}>
              {agent.resultPreview ??
                (agent.state === "running" ? "Still running…" : agent.state === "queued" ? "Not started." : "No output recorded.")}
            </div>
          </section>
        </div>
      </div>
    </>
  );
}
