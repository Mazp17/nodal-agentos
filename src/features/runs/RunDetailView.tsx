import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { projectIdOf, useRun, type RunsState } from "../../domain/hooks/runs";
import type { Run, RunLight, Verdict } from "../../domain/types";
import { formatDateTime, formatDuration, formatTokens } from "../../lib/format";
import { useRunActions } from "./actions";
import { AGENT_STATUS, AgentTranscript, modelName } from "./AgentTranscript";
import { classifyLaunchError, LaunchBlockerNotice, useLaunchBlocker } from "./LaunchBlockerNotice";
import { RunBadge } from "./RunBadge";
import {
  executorKindLabel,
  executorLabel,
  OUTCOME_LABEL,
  OUTCOME_TONE,
  prNumber,
  runName,
  runTaskRef,
  type RunView,
} from "./status";
import type { AgentInfo, RunDetail, RunResult } from "./types";
import "./run-detail.css";
import "./runs.css";

export interface RunDetailViewProps {
  runId: string;
  onBack: () => void;
  onOpenTask: (taskId: string) => void;
  /** Para saltar al revisor de este run (o al run que revisa). */
  onOpenRun?: (runId: string) => void;
  /** Abre el diff en el drawer que monta el shell. */
  onOpenDiff: (runId: string) => void;
}

function openPr(url: string) {
  openUrl(url).catch((e) => console.error("open PR", e));
}

function lastAction(a: AgentInfo): string {
  if (!a.lastToolName) return "—";
  return a.lastToolSummary ? `${a.lastToolName} ${a.lastToolSummary}` : a.lastToolName;
}

/** Pantalla "Run detail": cabecera, avisos, fases y subagentes (o sesión), y resultado. */
export function RunDetailView({ runId, onBack, onOpenTask, onOpenRun, onOpenDiff }: RunDetailViewProps) {
  const { view, full, state } = useRun(runId);
  const actions = useRunActions();
  const [agentIdx, setAgentIdx] = useState<number | null>(null);
  const blocker = useLaunchBlocker(view);

  if (!view) {
    return (
      <div className="rd">
        <button type="button" className="rd-back" onClick={onBack}>
          ← Back
        </button>
        <p className="rd-result-note">{state.loaded && full === null ? "This run no longer exists." : "Loading run…"}</p>
      </div>
    );
  }

  const run = view.run;
  const task = run.taskId ? state.tasks.get(run.taskId) : undefined;
  const pid = projectIdOf(run, state.tasks, state.repos);
  const project = pid ? state.projects.get(pid) : undefined;
  const repo = run.repoId ? state.repos.get(run.repoId) : task ? state.repos.get(task.repoId) : undefined;
  const ref = runTaskRef(run, task, project);
  const name = runName(run, task, project);
  const detail = view.detail;
  const isWorkflow = run.executor.kind === "workflow";
  const branch = run.branch ?? detail?.result?.branch ?? task?.worktree?.branch ?? null;
  const live = view.phase === "running" || view.phase === "waiting" || view.phase === "starting";
  const ended = view.tab === "finished" || view.tab === "failed";
  // `error` también guarda notas ("Stopped before finishing."...): banner solo si falló.
  const genericError = view.phase === "failed" && run.error && !classifyLaunchError(run.error, run.cwd) && !blocker ? run.error : null;
  const parent = run.parentRunId ? state.byId.get(run.parentRunId) : undefined;

  const agent = agentIdx != null ? detail?.agents[agentIdx] : undefined;
  const phases = detail?.phases ?? [];

  return (
    <div className="rd">
      <div className="rd-head">
        <div className="rd-head-main">
          <nav className="rd-crumbs" aria-label="Breadcrumb">
            <button type="button" className="rd-back" onClick={onBack}>
              ← Back
            </button>
            <span className="rd-crumb-sep">·</span>
            {project && (
              <>
                <span className="proj-swatch" style={{ background: project.color }} aria-hidden />
                <span>{project.name}</span>
                <span className="rd-crumb-sep">/</span>
              </>
            )}
            {repo && (
              <>
                <span>{repo.name}</span>
                <span className="rd-crumb-sep">/</span>
              </>
            )}
            {task ? (
              <button type="button" className="rd-crumb-task" onClick={() => onOpenTask(task.id)}>
                {ref.key ?? task.title}
              </button>
            ) : (
              <span>{ref.title}</span>
            )}
            {run.claudeRunId && (
              <>
                <span className="rd-crumb-sep">/</span>
                <span className="mono rd-crumb-id">{run.claudeRunId}</span>
              </>
            )}
          </nav>
          <div className="rd-title-row">
            <h1 className="rd-title">{ref.title}</h1>
            <RunBadge run={view} />
            {run.kind === "review" && <span className="rd-chip">Review</span>}
            {run.legacyLabel && <span className="rd-chip" title={run.legacyLabel}>Migrated</span>}
          </div>
          <div className="rd-meta">
            <span>
              Repo <span className="rd-meta-v">{repo?.name ?? "—"}</span>
            </span>
            <span>
              Branch <span className="rd-meta-v mono">{branch ?? "—"}</span>
            </span>
            <span>
              {executorKindLabel(run.executor)} <span className="rd-meta-v">{executorLabel(run.executor)}</span>
            </span>
            {run.launchedAt != null && (
              <span>
                Started <span className="rd-meta-v">{formatDateTime(run.launchedAt)}</span>
              </span>
            )}
            <span>
              Elapsed <span className="rd-meta-v num">{formatDuration(view.durationMs)}</span>
            </span>
            {isWorkflow && (
              <span>
                Tokens <span className="rd-meta-v num">{formatTokens(view.tokens)}</span>
              </span>
            )}
          </div>
        </div>
        <div className="rd-actions">
          <RunActionsBar
            view={view}
            state={state}
            name={name}
            actions={actions}
            onDiff={() => onOpenDiff(run.id)}
            onBack={onBack}
          />
        </div>
      </div>

      {view.phase === "waiting" && (
        // Claude Code no expone forma de responder desde fuera de la sesión: se responde con attach.
        <div className="rd-alert rd-alert-amber" role="status">
          <span className="dot dot-lg pulse" aria-hidden />
          <div className="rd-alert-body">
            <span className="rd-alert-title">
              {view.waitingFor === "permission prompt"
                ? "This run is waiting for a permission prompt"
                : "This run is waiting for your input"}
            </span>
            <span className="rd-alert-text">
              {view.waitingFor === "permission prompt"
                ? "An agent asked to use a tool outside the allowlist. Claude Code can't answer prompts programmatically: attach to the session and respond there."
                : `Blocked on: ${view.waitingFor}. Attach to respond.`}
            </span>
            {run.claudeRunId && <span className="rd-alert-mono">claude attach {run.claudeRunId}</span>}
          </div>
          {run.claudeRunId && (
            <button type="button" className="btn btn-amber" disabled={actions.busy} onClick={() => void actions.attach(view)}>
              Attach to respond
            </button>
          )}
        </div>
      )}

      {view.phase === "awaiting" && (
        <div className="rd-alert rd-alert-amber" role="status">
          <span className="dot dot-lg" aria-hidden />
          <div className="rd-alert-body">
            <span className="rd-alert-title">Migrated run awaiting confirmation</span>
            <span className="rd-alert-text">
              This run was queued in a previous version ({run.legacyLabel}). It won't start on its own: confirm it to
              queue it again, or remove it.
            </span>
          </div>
        </div>
      )}

      <LaunchBlockerNotice view={view} blocker={blocker} />
      {genericError && (
        <div className="rd-alert rd-alert-red" role="alert">
          <span className="dot dot-lg" aria-hidden />
          <div className="rd-alert-body">
            <span className="rd-alert-title">{run.launchedAt == null ? "Launch failed" : "Run error"}</span>
            <span className="rd-alert-text">{genericError}</span>
          </div>
        </div>
      )}

      {isWorkflow && phases.length > 0 && <Timeline view={view} detail={detail!} />}

      <div className="rd-grid">
        {isWorkflow ? (
          <SubagentsPanel view={view} detail={detail} onOpen={setAgentIdx} />
        ) : (
          <SessionPanel view={view} full={full} live={live} />
        )}
        <div className="rd-side">
          <ResultCard view={view} detail={detail} branch={branch} ended={ended} />
          {run.kind === "work" && view.review && (
            <VerdictCard review={view.review} state={state} onOpenRun={onOpenRun} />
          )}
          {run.kind === "review" && parent && onOpenRun && (
            <button type="button" className="btn rd-parent" onClick={() => onOpenRun(parent.run.id)}>
              Open the reviewed run ({executorLabel(parent.run.executor)})
            </button>
          )}
        </div>
      </div>

      {agent && (
        <AgentTranscript
          agent={agent}
          phaseNum={phases.findIndex((p) => p.title === agent.phase) + 1 || null}
          view={view}
          workflowId={detail?.workflowId ?? null}
          taskRef={ref.key}
          onClose={() => setAgentIdx(null)}
        />
      )}
    </div>
  );
}

function RunActionsBar({
  view,
  state,
  name,
  actions,
  onDiff,
  onBack,
}: {
  view: RunView;
  state: RunsState;
  name: string;
  actions: ReturnType<typeof useRunActions>;
  onDiff: () => void;
  onBack: () => void;
}) {
  const run = view.run;
  const task = run.taskId ? state.tasks.get(run.taskId) : undefined;
  const canDiff = run.launchedAt != null;
  const diffBtn = canDiff && (
    <button type="button" className="btn" onClick={onDiff}>
      View diff
    </button>
  );
  switch (view.phase) {
    case "awaiting":
      return (
        <>
          <button
            type="button"
            className="btn btn-danger"
            disabled={actions.busy}
            onClick={async () => {
              if (await actions.remove(view, name)) onBack();
            }}
          >
            Remove from queue
          </button>
          <button type="button" className="btn btn-primary" disabled={actions.busy} onClick={() => void actions.confirm(view, name)}>
            Confirm
          </button>
        </>
      );
    case "queued":
      return (
        <button
          type="button"
          className="btn btn-danger"
          disabled={actions.busy}
          onClick={async () => {
            if (await actions.remove(view, name)) onBack();
          }}
        >
          Remove from queue
        </button>
      );
    case "launching":
      return null;
    case "starting":
    case "running":
    case "waiting":
      return (
        <>
          <button type="button" className="btn btn-danger" disabled={actions.busy} onClick={() => void actions.stop(view, name)}>
            Stop
          </button>
          {diffBtn}
          {run.claudeRunId && (
            <button
              type="button"
              className={`btn ${view.phase === "waiting" ? "btn-primary" : ""}`}
              disabled={actions.busy}
              onClick={() => void actions.attach(view)}
            >
              Attach
            </button>
          )}
        </>
      );
    case "finished":
    case "failed":
    case "canceled": {
      const pr = run.prUrl;
      const again = run.taskId && run.kind === "work" && (
        <button
          type="button"
          className={`btn ${pr ? "" : "btn-primary"}`}
          disabled={actions.busy}
          onClick={() => void actions.runAgain(view, task, name)}
        >
          Run again
        </button>
      );
      return (
        <>
          {diffBtn}
          {again}
          {pr && (
            <button type="button" className="btn btn-primary" onClick={() => openPr(pr)}>
              Open PR {prNumber(pr) ?? ""} ↗
            </button>
          )}
        </>
      );
    }
  }
}

function Timeline({ view, detail }: { view: RunView; detail: RunDetail }) {
  const phases = detail.phases;
  const cur = detail.currentPhaseIndex;
  const finished = view.phase === "finished";
  const stopped = view.tab === "failed";
  return (
    <div className="panel rd-timeline">
      <ol className="tl" style={{ gridTemplateColumns: `repeat(${phases.length}, minmax(0, 1fr))` }}>
        {phases.map((ph, i) => {
          const num = i + 1;
          const done = finished || (cur != null && num < cur);
          const isCur = !finished && cur === num;
          const st = done ? "done" : isCur ? (stopped ? "failed" : "current") : "pending";
          return (
            <li key={ph.title + i} className={`tl-step tl-${st} tone-${view.tone}`} aria-current={isCur ? "step" : undefined}>
              <div className="tl-mark-row">
                <span className="tl-mark">{done ? "✓" : st === "failed" ? "✕" : num}</span>
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
  );
}

function SubagentsPanel({
  view,
  detail,
  onOpen,
}: {
  view: RunView;
  detail: RunDetail | null | undefined;
  onOpen: (idx: number) => void;
}) {
  const phases = detail?.phases ?? [];
  const cur = detail?.currentPhaseIndex ?? null;
  const finished = view.phase === "finished";

  // Subagentes por fase (en el orden del workflow); los que no tienen fase van al final.
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
  const active = detail?.agents.filter((a) => a.state === "running").length ?? 0;
  const summary = detail ? `${detail.agentCount} agent${detail.agentCount === 1 ? "" : "s"}${active ? ` · ${active} active` : ""}` : "";

  let empty: string | null = null;
  if (!view.run.sessionId) empty = view.tab === "queued" ? "Agents appear once the run starts." : "No agent data for this run.";
  else if (detail === undefined) empty = "Loading agents…";
  else if (detail === null) empty = "This session didn't launch a workflow.";
  else if (detail.agents.length === 0) empty = "No subagents yet.";

  return (
    <div className="panel rd-agents">
      <div className="rd-panel-head">
        <span className="rd-panel-title">Subagents</span>
        <span className="rd-panel-sub">{summary}</span>
        {!empty && <span className="rd-panel-hint">Click a row to open its transcript</span>}
      </div>
      {empty ? (
        <div className="rd-agents-empty">{empty}</div>
      ) : (
        groups
          .filter((g) => g.agents.length > 0 || g.num !== null)
          .map((g) => {
            const isCur = !finished && g.num != null && g.num === cur;
            const past = finished || (g.num != null && cur != null && g.num < cur);
            return (
              <div key={g.title + (g.num ?? "x")} className="rd-group">
                <div className={`rd-group-head ${isCur ? "current" : past ? "past" : ""}`}>
                  {g.num != null && <span className="rd-group-num">{String(g.num).padStart(2, "0")}</span>}
                  <span className="rd-group-name">{g.title}</span>
                  <span className="rd-group-note">
                    {isCur ? "current" : g.agents.length ? `${g.agents.length} agent${g.agents.length === 1 ? "" : "s"}` : past ? "" : "Pending"}
                  </span>
                </div>
                {g.agents.map(([a, idx]) => {
                  const st = AGENT_STATUS[a.state];
                  return (
                    <button key={a.agentId ?? `${a.label}-${idx}`} type="button" className="rd-cols rd-agent" onClick={() => onOpen(idx)}>
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
          })
      )}
      {detail && detail.workflowCount > 1 && (
        <p className="rd-agents-empty">This session ran {detail.workflowCount} workflows; showing the latest.</p>
      )}
    </div>
  );
}

/**
 * Runs de agente o de Claude: no hay fases ni subagentes que leer. Se muestra el prompt
 * que armó Nodal y el reporte final (el resumen que devolvió el agente en su último mensaje).
 */
function SessionPanel({ view, full, live }: { view: RunView; full: Run | null | undefined; live: boolean }) {
  const run = view.run;
  return (
    <div className="panel rd-session">
      <div className="rd-panel-head">
        <span className="rd-panel-title">Session</span>
        <span className="rd-panel-sub">
          {executorKindLabel(run.executor)} · {executorLabel(run.executor)}
          {run.options.model ? ` · ${run.options.model}` : ""}
        </span>
      </div>
      <div className="rd-session-body">
        <section className="ap-section">
          <h3 className="section-label">Last message</h3>
          {run.summary ? (
            <div className="rd-session-msg">{run.summary}</div>
          ) : (
            <p className="rd-result-note">
              {live
                ? "The agent is working. Its final report shows up here when the session ends; attach to follow it live."
                : view.tab === "queued"
                  ? "The session hasn't started yet."
                  : "The session ended without a final report."}
            </p>
          )}
        </section>
        {full?.extraInstructions && (
          <section className="ap-section">
            <h3 className="section-label">Extra instructions</h3>
            <div className="tr-prompt">{full.extraInstructions}</div>
          </section>
        )}
        <details className="rd-res-raw">
          <summary>Prompt</summary>
          <pre>{full ? full.prompt : "Loading…"}</pre>
        </details>
      </div>
    </div>
  );
}

function StatusLine({ tone, label, pulse }: { tone: string; label: string; pulse?: boolean }) {
  return (
    <div className={`rd-result-status tone-${tone}`}>
      <span className={`dot dot-lg ${pulse ? "pulse" : ""}`} aria-hidden />
      {label}
    </div>
  );
}

function CriteriaList({ unmet, label = "Acceptance criteria" }: { unmet: string[]; label?: string }) {
  return (
    <div className="rd-res-sec">
      <span className="rd-res-label">{label}</span>
      {unmet.length === 0 ? (
        <div className="rd-res-item">
          <span className="rd-res-mark tone-ok" aria-hidden>
            ✓
          </span>
          All criteria met
        </div>
      ) : (
        <ul className="rd-res-list">
          {unmet.map((u, i) => (
            <li key={i} className="rd-res-item">
              <span className="rd-res-mark tone-danger" aria-hidden>
                ✕
              </span>
              <span>{u}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function NitsList({ nits }: { nits: string[] }) {
  if (!nits.length) return null;
  return (
    <div className="rd-res-sec">
      <span className="rd-res-label">Reviewer nits</span>
      <ul className="rd-res-list">
        {nits.map((n, i) => (
          <li key={i} className="rd-res-item rd-res-nit">
            <span className="rd-res-mark tone-muted" aria-hidden>
              ·
            </span>
            <span>{n}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

function VerdictBody({ verdict }: { verdict: Verdict }) {
  return (
    <>
      <StatusLine tone={verdict.pass ? "ok" : "danger"} label={verdict.pass ? "Review passed" : "Review failed"} />
      {verdict.summary && <p className="rd-result-text">{verdict.summary}</p>}
      <CriteriaList unmet={verdict.unmet} />
      <NitsList nits={verdict.nits} />
    </>
  );
}

/** JSON con lo que Nodal leyó del run (o el `result` del workflow). */
function rawResult(run: RunLight, workflowResult: RunResult | null | undefined): string | null {
  if (workflowResult?.raw) return workflowResult.raw;
  const out: Record<string, unknown> = {};
  if (run.outcome) out.outcome = run.outcome;
  if (run.summary) out.summary = run.summary;
  if (run.prUrl) out.pr = run.prUrl;
  if (run.branch) out.branch = run.branch;
  if (run.verdict) out.verdict = run.verdict;
  if (run.error) out.error = run.error;
  return Object.keys(out).length ? JSON.stringify(out, null, 2) : null;
}

function ResultCard({
  view,
  detail,
  branch,
  ended,
}: {
  view: RunView;
  detail: RunDetail | null | undefined;
  branch: string | null;
  ended: boolean;
}) {
  const run = view.run;
  const [rawOpen, setRawOpen] = useState(false);

  let body;
  if (!ended) {
    const note =
      view.phase === "awaiting"
        ? "Confirm the run to queue it again."
        : view.phase === "queued"
          ? `Waiting in the global queue${view.queuePos != null ? ` at position #${view.queuePos}` : ""}.`
          : `The result appears when the run finishes.${view.phaseName ? ` Currently in ${view.phaseName}.` : ""}`;
    body = <p className="rd-result-note">{note}</p>;
  } else {
    const wr = detail?.source === "final" ? detail.result : null;
    const outcome = run.outcome ?? "unknown";
    const pr = run.prUrl ?? wr?.pr ?? null;
    const unmet = wr?.unmetAcceptance ?? null;
    const nits = wr?.nits ?? [];
    const raw = rawResult(run, wr);
    body = (
      <>
        {run.kind === "review" && run.verdict ? (
          <VerdictBody verdict={run.verdict} />
        ) : (
          <>
            <StatusLine
              tone={view.tab === "failed" && !run.outcome ? "danger" : OUTCOME_TONE[outcome]}
              label={view.tab === "failed" && !run.outcome ? view.label : OUTCOME_LABEL[outcome]}
            />
            {run.summary && <p className="rd-result-text">{run.summary}</p>}
          </>
        )}
        {run.error && view.phase !== "failed" && <p className="rd-result-note">{run.error}</p>}
        {pr && (
          <button type="button" className="rd-pr" onClick={() => openPr(pr)}>
            <span className="rd-pr-title">Pull request {prNumber(pr) ?? ""} ↗</span>
            {branch && <span className="rd-pr-sub mono">{branch}</span>}
          </button>
        )}
        <div className="rd-res-row">
          <span className="rd-res-label">Branch</span>
          <span className="rd-res-mono ellipsis">{branch ?? "—"}</span>
        </div>
        {wr?.where && (
          <div className="rd-res-sec">
            <span className="rd-res-label">Stopped at</span>
            <span className="rd-res-item">{wr.where}</span>
          </div>
        )}
        {unmet && <CriteriaList unmet={unmet} />}
        <NitsList nits={nits} />
        {raw && (
          <div className="rd-raw">
            <button type="button" className="rd-raw-toggle" aria-expanded={rawOpen} onClick={() => setRawOpen(!rawOpen)}>
              <span aria-hidden>{rawOpen ? "▾" : "▸"}</span>
              Raw result
            </button>
            {rawOpen && <pre>{raw}</pre>}
          </div>
        )}
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
    </div>
  );
}

/** Veredicto del revisor de Nodal sobre un run de trabajo. */
function VerdictCard({ review, state, onOpenRun }: { review: RunLight; state: RunsState; onOpenRun?: (id: string) => void }) {
  const rv = state.byId.get(review.id);
  return (
    <div className="panel rd-result">
      <div className="rd-verdict-head">
        <h2 className="rd-result-label">Reviewer verdict</h2>
        <span className="rd-panel-sub">{executorLabel(review.executor)}</span>
        {onOpenRun && (
          <button type="button" className="btn btn-xs rd-verdict-open" onClick={() => onOpenRun(review.id)}>
            Open review
          </button>
        )}
      </div>
      {review.verdict ? (
        <VerdictBody verdict={review.verdict} />
      ) : rv ? (
        <>
          <StatusLine tone={rv.tone} label={rv.label} pulse={rv.pulse} />
          <p className="rd-result-note">
            {rv.tab === "finished" || rv.tab === "failed" ? "The reviewer didn't return a verdict." : "The verdict appears when the review finishes."}
          </p>
        </>
      ) : (
        <p className="rd-result-note">The verdict appears when the review finishes.</p>
      )}
    </div>
  );
}
