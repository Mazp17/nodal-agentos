import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { projectIdOf, useRun, type RunsState } from "../../domain/hooks/runs";
import type { Run, RunLight, Verdict } from "../../domain/types";
import { formatDateTime, formatDuration, formatTokens } from "../../lib/format";
import { SafeMarkdown } from "../../ui/Markdown";
import { useToast } from "../../ui/Toasts";
import { ExecutorAvatar } from "../executors";
import { useRunActions } from "./actions";
import {
  AGENT_STATUS,
  AgentTranscript,
  DEFAULT_LIMIT,
  FULL_LIMIT,
  modelName,
  TranscriptConversation,
  useRunTranscript,
} from "./AgentTranscript";
import { classifyLaunchError, LaunchBlockerNotice, useLaunchBlocker } from "./LaunchBlockerNotice";
import { RunBadge } from "./RunBadge";
import {
  executorKindLabel,
  executorLabel,
  OUTCOME_LABEL,
  OUTCOME_TONE,
  phaseProgress,
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
  /** To jump to this run's reviewer (or to the run it reviews). */
  onOpenRun?: (runId: string) => void;
  /** Opens the diff in the drawer mounted by the shell. */
  onOpenDiff: (runId: string) => void;
}

function openPr(url: string) {
  openUrl(url).catch((e) => console.error("open PR", e));
}

function lastAction(a: AgentInfo): string {
  if (!a.lastToolName) return "—";
  return a.lastToolSummary ? `${a.lastToolName} ${a.lastToolSummary}` : a.lastToolName;
}

/** "Run detail" screen: header, notices, phases and subagents (or session), and result. */
export function RunDetailView({ runId, onBack, onOpenTask, onOpenRun, onOpenDiff }: RunDetailViewProps) {
  const { view, full, state } = useRun(runId);
  const actions = useRunActions();
  const [agentIdx, setAgentIdx] = useState<number | null>(null);
  const blocker = useLaunchBlocker(view);
  const toast = useToast();

  if (!view) {
    return (
      <div className="rd rd-missing">
        <button type="button" className="btn btn-xs btn-ghost rd-back" onClick={onBack}>
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
  // `error` also stores notes ("Stopped before finishing."...): banner only if it failed.
  const genericError = view.phase === "failed" && run.error && !classifyLaunchError(run.error, run.cwd) && !blocker ? run.error : null;
  const parent = run.parentRunId ? state.byId.get(run.parentRunId) : undefined;

  const agent = agentIdx != null ? detail?.agents[agentIdx] : undefined;
  const phases = detail?.phases ?? [];
  const nits = (detail?.source === "final" ? detail.result?.nits : null) ?? [];

  const copyRunId = async (id: string) => {
    try {
      await navigator.clipboard.writeText(id);
      toast("Run ID copied", id, "ok");
    } catch (e) {
      toast("Couldn't copy the run ID", String(e), "danger");
    }
  };

  return (
    <div className="rd">
      <div className="rd-bar">
        <nav className="rd-crumbs" aria-label="Breadcrumb">
          <button type="button" className="btn btn-xs btn-ghost" onClick={onBack}>
            ← Back
          </button>
          <span className="rd-crumb-sep" aria-hidden>
            ·
          </span>
          {project && (
            <>
              <span className="proj-swatch" style={{ background: project.color }} aria-hidden />
              <span className="rd-crumb-text">{project.name}</span>
              <span className="rd-crumb-sep" aria-hidden>
                /
              </span>
            </>
          )}
          {repo && (
            <>
              <span className="rd-crumb-text">{repo.name}</span>
              <span className="rd-crumb-sep" aria-hidden>
                /
              </span>
            </>
          )}
          {task ? (
            <button type="button" className="rd-crumb-task" onClick={() => onOpenTask(task.id)}>
              {ref.key ?? task.title}
            </button>
          ) : (
            <span className="rd-crumb-text ellipsis">{ref.title}</span>
          )}
          {run.claudeRunId && (
            <>
              <span className="rd-crumb-sep" aria-hidden>
                /
              </span>
              <button
                type="button"
                className="btn btn-xs btn-ghost rd-crumb-id"
                title={`Copy Claude run ID (${run.claudeRunId})`}
                aria-label={`Copy Claude run ID ${run.claudeRunId}`}
                onClick={() => void copyRunId(run.claudeRunId!)}
              >
                <span className="mono ellipsis">{run.claudeRunId}</span>
                <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.2" aria-hidden>
                  <rect x="3.5" y="3.5" width="7" height="7" rx="1.5" />
                  <path d="M8.5 3.5v-1a1 1 0 0 0-1-1h-5a1 1 0 0 0-1 1v5a1 1 0 0 0 1 1h1" />
                </svg>
              </button>
            </>
          )}
        </nav>
        <div className="rd-actions">
          {task && (
            <button type="button" className="btn btn-sm" onClick={() => onOpenTask(task.id)}>
              Open task
            </button>
          )}
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

      <div className="rd-scroll">
        <div className="rd-body">
          <div className="rd-hero">
            <h1 className="rd-title">{ref.title}</h1>
            <div className="rd-chips">
              <RunBadge run={view} className="rd-status" />
              {run.kind === "review" && <span className="rd-chip">Review</span>}
              {run.legacyLabel && (
                <span className="rd-chip" title={run.legacyLabel}>
                  Migrated
                </span>
              )}
              <span className="rd-chip" title={`${executorKindLabel(run.executor)} · ${executorLabel(run.executor)}`}>
                <ExecutorAvatar executor={run.executor} />
                <span className="mono">{executorLabel(run.executor)}</span>
              </span>
              {repo && <span className="rd-chip mono">{repo.name}</span>}
              <span className="rd-chip num">
                <span className="rd-chip-k">Elapsed</span>
                {formatDuration(view.durationMs)}
              </span>
              {(isWorkflow || run.tokens != null) && (
                <span className="rd-chip num">
                  <span className="rd-chip-k">Tokens</span>
                  {formatTokens(isWorkflow ? view.tokens : run.tokens)}
                </span>
              )}
            </div>
          </div>

          {view.phase === "waiting" && (
            // Claude Code exposes no way to respond from outside the session: respond via attach.
            <div className="rd-alert rd-alert-amber" role="status">
              <span className="rd-alert-mark" aria-hidden>
                !
              </span>
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
                <button type="button" className="btn btn-primary" disabled={actions.busy} onClick={() => void actions.attach(view)}>
                  Attach to respond
                </button>
              )}
            </div>
          )}

          {view.phase === "awaiting" && (
            <div className="rd-alert rd-alert-amber" role="status">
              <span className="rd-alert-mark" aria-hidden>
                !
              </span>
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
              <span className="rd-alert-mark" aria-hidden>
                ✕
              </span>
              <div className="rd-alert-body">
                <span className="rd-alert-title">{run.launchedAt == null ? "Launch failed" : "Run error"}</span>
                <span className="rd-alert-text">{genericError}</span>
              </div>
            </div>
          )}

          {ended && <ResultCard view={view} detail={detail} branch={branch} />}

          {live && isWorkflow && view.phaseTotal > 0 && <NowCard view={view} detail={detail} />}

          {isWorkflow && phases.length > 0 && <Timeline view={view} detail={detail!} />}

          {isWorkflow ? (
            <SubagentsPanel view={view} detail={detail} onOpen={setAgentIdx} />
          ) : (
            <SessionPanel view={view} full={full} live={live} />
          )}

          <div className="rd-cards">
            {!ended && <PendingResult view={view} />}
            {run.kind === "work" && view.review && <VerdictCard review={view.review} state={state} onOpenRun={onOpenRun} />}
            {ended && nits.length > 0 && (
              <div className="rd-card">
                <NitsList nits={nits} />
              </div>
            )}
            <DetailsCard view={view} detail={detail} branch={branch} ended={ended} />
            {run.kind === "review" && parent && onOpenRun && (
              <button type="button" className="btn rd-parent" onClick={() => onOpenRun(parent.run.id)}>
                Open the reviewed run ({executorLabel(parent.run.executor)})
              </button>
            )}
          </div>
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
    <button type="button" className="btn btn-sm" onClick={onDiff}>
      View diff
    </button>
  );
  switch (view.phase) {
    case "awaiting":
      return (
        <>
          <button
            type="button"
            className="btn btn-sm btn-danger"
            disabled={actions.busy}
            onClick={async () => {
              if (await actions.remove(view, name)) onBack();
            }}
          >
            Remove from queue
          </button>
          <button type="button" className="btn btn-sm btn-primary" disabled={actions.busy} onClick={() => void actions.confirm(view, name)}>
            Confirm
          </button>
        </>
      );
    case "queued":
      return (
        <button
          type="button"
          className="btn btn-sm btn-danger"
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
          <button type="button" className="btn btn-sm btn-danger" disabled={actions.busy} onClick={() => void actions.stop(view, name)}>
            Stop
          </button>
          {diffBtn}
          {run.claudeRunId && (
            <button
              type="button"
              className={`btn btn-sm ${view.phase === "waiting" ? "btn-primary" : ""}`}
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
          className={`btn btn-sm ${pr ? "" : "btn-primary"}`}
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
            <button type="button" className="btn btn-sm btn-primary" onClick={() => openPr(pr)}>
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
    <section className="rd-timeline" aria-label="Phases">
      <span className="section-label">Phases</span>
      <ol className="tl" style={{ gridTemplateColumns: `repeat(${phases.length}, minmax(56px, 1fr))` }}>
        {phases.map((ph, i) => {
          const num = i + 1;
          const done = finished || (cur != null && num < cur);
          const isCur = !finished && cur === num;
          const st = done ? "done" : isCur ? (stopped ? "failed" : "current") : "pending";
          const label = done ? "done" : st === "failed" ? "stopped" : isCur ? "current" : "pending";
          return (
            <li key={ph.title + i} className={`tl-step tl-${st} tone-${view.tone}`} aria-current={isCur ? "step" : undefined}>
              <span className="tl-bar" aria-hidden />
              <span className="tl-name" title={ph.detail ?? ph.title}>
                {ph.title}
                <span className="sr-only"> ({label})</span>
              </span>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

/** Live workflow: how far it got and what the current phase is doing. */
function NowCard({ view, detail }: { view: RunView; detail: RunDetail | null | undefined }) {
  const { pct } = phaseProgress(view);
  const cur = view.phaseIndex != null ? detail?.phases[view.phaseIndex - 1] : undefined;
  const msg = cur?.detail ?? (view.phaseName ? `Currently in ${view.phaseName}.` : null);
  return (
    <div className={`rd-card rd-now tone-${view.tone}`} role="region" aria-label="Current state">
      <div className="rd-now-head">
        <span className="section-label">Now</span>
        <span className="rd-now-pct num">
          {view.phaseIndex != null ? `Phase ${view.phaseIndex}/${view.phaseTotal} · ` : ""}
          {pct}%
        </span>
      </div>
      <span className="rd-now-bar" aria-hidden>
        <span style={{ width: `${pct}%` }} />
      </span>
      {msg && <div className="rd-now-msg">{msg}</div>}
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
  // The run is waiting on a permission prompt: the agent that is still running is the one asking.
  const waiting = view.phase === "waiting";
  const stopped = view.tab === "failed";

  // Subagents by phase (in workflow order); those without a phase go last.
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
  const all = detail?.agents ?? [];
  const done = all.filter((a) => a.state === "done").length;
  const active = all.filter((a) => a.state === "running").length;
  const summary = detail
    ? `${plural(detail.agentCount, "agent")} · ${done} done${active ? ` · ${active} active` : ""}`
    : "";

  let empty: string | null = null;
  if (!view.run.sessionId) empty = view.tab === "queued" ? "Agents appear once the run starts." : "No agent data for this run.";
  else if (detail === undefined) empty = "Loading agents…";
  else if (detail === null) empty = "This session didn't launch a workflow.";
  else if (detail.agents.length === 0) empty = "No subagents yet.";

  const shown = groups.filter((g) => g.agents.length > 0 || g.num !== null);
  return (
    <div className="rd-card rd-panel rd-agents">
      <div className="rd-panel-head">
        <span className="rd-panel-title">Subagents</span>
        <span className="rd-panel-sub">{summary}</span>
        {!empty && <span className="rd-panel-hint">Click a row to open its transcript</span>}
      </div>
      {empty ? (
        <div className="rd-agents-empty">{empty}</div>
      ) : (
        <div className="rd-phases">
          {shown.map((g, gi) => {
            // Agents with no phase ("Other") count as done once the run ends, else as current.
            const running = g.agents.filter(([a]) => a.state === "running").length;
            // A phase with running agents is current even if the workflow didn't report its index.
            const st: "done" | "current" | "pending" =
              finished || (g.num != null && cur != null && g.num < cur)
                ? "done"
                : g.num == null || g.num === cur || running > 0
                  ? "current"
                  : "pending";
            const took = g.agents.reduce((t, [a]) => t + (a.durationMs ?? 0), 0);
            const count = plural(g.agents.length, "agent");
            const meta =
              g.agents.length === 0
                ? st === "pending"
                  ? "Pending"
                  : ""
                : st === "done"
                  ? `${count} · ${formatDuration(took)}`
                  : st === "pending"
                    ? count
                    : stopped
                      ? `${count} · Stopped`
                      : `${count} · ${running ? `${running} active` : "wrapping up"}`;
            return (
              <div key={g.title + (g.num ?? "x")} className={`rd-phase rd-phase-${st} tone-${view.tone}`}>
                <div className="rd-phase-rail" aria-hidden>
                  <span className="rd-phase-mark">{st === "done" ? "✓" : st === "current" && stopped ? "✕" : ""}</span>
                  {gi < shown.length - 1 && <span className="rd-phase-line" />}
                </div>
                <div className="rd-phase-body">
                  <div className="rd-phase-head">
                    <span className="rd-phase-name">{g.title}</span>
                    {meta && <span className="rd-phase-meta">{meta}</span>}
                  </div>
                  {g.agents.length > 0 && (
                    <div className="rd-phase-agents">
                      {g.agents.map(([a, idx]) => {
                        const perm = waiting && a.state === "running";
                        const status = perm ? { label: "Needs permission", tone: "warn" } : AGENT_STATUS[a.state];
                        const showStatus = perm || !(a.state === "done" || a.state === "queued");
                        return (
                          <button
                            key={a.agentId ?? `${a.label}-${idx}`}
                            type="button"
                            className={`rd-agent tone-${status.tone}`}
                            onClick={() => onOpen(idx)}
                          >
                            <span className={`rd-agent-dot ${a.state === "running" && !perm ? "pulse" : ""}`} aria-hidden />
                            {!showStatus && <span className="sr-only">{status.label}</span>}
                            <span className="rd-agent-label ellipsis">{a.label}</span>
                            <span className="rd-agent-model ellipsis">{modelName(a.model)}</span>
                            <span className="rd-agent-last">
                              {showStatus && <span className="rd-agent-st">{status.label}</span>}
                              <span className="ellipsis">{lastAction(a)}</span>
                            </span>
                            <span className="text-right num rd-agent-num">{formatTokens(a.tokens)}</span>
                            <span className="text-right num rd-agent-num">{formatDuration(a.durationMs)}</span>
                            <span className="rd-agent-chev" aria-hidden>
                              ›
                            </span>
                          </button>
                        );
                      })}
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      )}
      {detail && detail.workflowCount > 1 && (
        <p className="rd-agents-empty">This session ran {detail.workflowCount} workflows; showing the latest.</p>
      )}
    </div>
  );
}

const plural = (n: number, word: string) => `${n} ${word}${n === 1 ? "" : "s"}`;

/**
 * Agent, Claude or reviewer runs: there are no phases or subagents to read. Shows the
 * final report, the prompt Nodal built and the main session's transcript
 * (`get_run_transcript`), which repeats while the session is still alive.
 */
function SessionPanel({ view, full, live }: { view: RunView; full: Run | null | undefined; live: boolean }) {
  const run = view.run;
  const [limit, setLimit] = useState(DEFAULT_LIMIT);
  const [promptOpen, setPromptOpen] = useState(false);
  // Only "running"/"waiting" change the file; "starting" has no session yet.
  const polling = view.phase === "running" || view.phase === "waiting";
  const load = useRunTranscript(run.sessionId ? run.id : null, polling, limit);
  const t = load.status === "ok" ? load.transcript : null;
  return (
    <div className="rd-card rd-panel rd-session">
      <div className="rd-panel-head">
        <span className="rd-panel-title">Session</span>
        <span className="rd-panel-sub">
          {executorKindLabel(run.executor)} · {executorLabel(run.executor)}
          {run.options.model ? ` · ${run.options.model}` : ""}
        </span>
        {polling && (
          <span className="rd-live">
            <span className="dot dot-sm pulse" aria-hidden />
            Live
          </span>
        )}
      </div>
      <div className="rd-session-body">
        <section className="ap-section">
          <h3 className="section-label">Last message</h3>
          {run.summary ? (
            <div className="rd-session-msg">{run.summary}</div>
          ) : (
            <p className="rd-result-note">
              {live
                ? "The agent is working. Its final report shows up here when the session ends; the transcript below follows it live."
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
        <div className="rd-raw">
          <button type="button" className="rd-raw-toggle" aria-expanded={promptOpen} onClick={() => setPromptOpen(!promptOpen)}>
            <span aria-hidden>{promptOpen ? "▾" : "▸"}</span>
            Prompt
            {full && <span className="rd-raw-meta num">{full.prompt.length.toLocaleString()} chars</span>}
          </button>
          {promptOpen && <pre>{full ? full.prompt : "Loading…"}</pre>}
        </div>
        {!run.sessionId ? null : load.status === "loading" ? (
          <p className="rd-result-note">Loading transcript…</p>
        ) : load.status === "error" ? (
          <p className="rd-result-note tr-error" role="alert">
            Couldn't load the transcript: {load.error}
          </p>
        ) : t === null ? (
          <p className="rd-result-note">No transcript file for this session yet.</p>
        ) : (
          <TranscriptConversation
            t={t}
            limit={limit}
            onMore={() => setLimit(FULL_LIMIT)}
            live={polling}
            view={view}
            waiting={view.phase === "waiting"}
          />
        )}
      </div>
    </div>
  );
}

function StatusLine({ tone, label, pulse }: { tone: string; label: string; pulse?: boolean }) {
  return (
    <div className={`rd-result-status tone-${tone}`}>
      <span className={`dot ${pulse ? "pulse" : ""}`} aria-hidden />
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
              <SafeMarkdown text={u} className="md-compact" breaks />
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
            <SafeMarkdown text={n} className="md-compact" breaks />
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
      {verdict.summary && <SafeMarkdown text={verdict.summary} breaks />}
      <CriteriaList unmet={verdict.unmet} />
      <NitsList nits={verdict.nits} />
    </>
  );
}

/** JSON with what Nodal read from the run (or the workflow's `result`). */
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

/** Ended run: outcome, summary, PR or branch, and unmet criteria. */
function ResultCard({ view, detail, branch }: { view: RunView; detail: RunDetail | null | undefined; branch: string | null }) {
  const run = view.run;
  const wr = detail?.source === "final" ? detail.result : null;
  const outcome = run.outcome ?? "unknown";
  const pr = run.prUrl ?? wr?.pr ?? null;
  const unmet = wr?.unmetAcceptance ?? null;
  const review = run.kind === "review" && run.verdict ? run.verdict : null;
  const failed = view.tab === "failed" && !run.outcome;
  const tone = review ? (review.pass ? "ok" : "danger") : failed ? "danger" : OUTCOME_TONE[outcome];
  const allMet = (review ? review.unmet : unmet)?.length === 0;
  const shownUnmet = review ? review.unmet : unmet;
  return (
    <div className={`rd-card rd-result tone-${tone}`} role="region" aria-label="Result">
      <StatusLine
        tone={tone}
        label={review ? (review.pass ? "Review passed" : "Review failed") : failed ? view.label : OUTCOME_LABEL[outcome]}
      />
      {review ? review.summary && <SafeMarkdown text={review.summary} breaks /> : run.summary && <SafeMarkdown text={run.summary} breaks />}
      {run.error && view.phase !== "failed" && <p className="rd-result-note">{run.error}</p>}
      <div className="rd-chips">
        {pr ? (
          <button type="button" className="rd-chip rd-chip-btn" onClick={() => openPr(pr)}>
            <span className="rd-pr-title">PR {prNumber(pr) ?? ""} ↗</span>
            {branch && <span className="rd-chip-sub mono ellipsis">{branch}</span>}
          </button>
        ) : (
          <span className="rd-chip">
            <span className="rd-chip-k">Branch</span>
            <span className="mono ellipsis">{branch ?? "—"}</span>
          </span>
        )}
        {allMet && (
          <span className="rd-chip">
            <span className="rd-res-mark tone-ok" aria-hidden>
              ✓
            </span>
            All criteria met
          </span>
        )}
      </div>
      {wr?.where && (
        <div className="rd-res-sec rd-res-split">
          <span className="rd-res-label">Stopped at</span>
          <span className="rd-res-item">{wr.where}</span>
        </div>
      )}
      {shownUnmet && shownUnmet.length > 0 && (
        <div className="rd-res-split">
          <CriteriaList unmet={shownUnmet} label="Unmet criteria" />
        </div>
      )}
      {review && <NitsList nits={review.nits} />}
    </div>
  );
}

/** Not ended yet: where the run is. */
function PendingResult({ view }: { view: RunView }) {
  const note =
    view.phase === "awaiting"
      ? "Confirm the run to queue it again."
      : view.phase === "queued"
        ? `Waiting in the global queue${view.queuePos != null ? ` at position #${view.queuePos}` : ""}.`
        : `The result appears when the run finishes.${view.phaseName ? ` Currently in ${view.phaseName}.` : ""}`;
  return (
    <div className="rd-card">
      <h2 className="section-label">Result</h2>
      <p className="rd-result-note">{note}</p>
    </div>
  );
}

/** Key facts about the run, and the raw result once it ends. */
function DetailsCard({
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
  const raw = ended ? rawResult(run, detail?.source === "final" ? detail.result : null) : null;
  const rows: { k: string; v: string; mono?: boolean }[] = [
    { k: executorKindLabel(run.executor), v: executorLabel(run.executor), mono: true },
    { k: "Branch", v: branch ?? "—", mono: true },
  ];
  if (run.options.model) rows.push({ k: "Model", v: run.options.model, mono: true });
  if (run.launchedAt != null) rows.push({ k: "Started", v: formatDateTime(run.launchedAt) });
  rows.push({ k: "Elapsed", v: formatDuration(view.durationMs) });
  if (detail) {
    rows.push({ k: "Agents", v: String(detail.agentCount) });
    rows.push({ k: "Tool calls", v: detail.totalToolCalls != null ? String(detail.totalToolCalls) : "—" });
    rows.push({ k: "Tokens", v: formatTokens(detail.totalTokens) });
  } else if (run.tokens != null) {
    rows.push({ k: "Tokens", v: formatTokens(run.tokens) });
  }
  return (
    <div className="rd-card">
      <h2 className="section-label">Details</h2>
      <dl className="rd-details">
        {rows.map((r) => (
          <div key={r.k}>
            <dt>{r.k}</dt>
            <dd className={`num ${r.mono ? "mono" : ""}`} title={r.v}>
              {r.v}
            </dd>
          </div>
        ))}
      </dl>
      {raw && (
        <div className="rd-raw">
          <button type="button" className="rd-raw-toggle" aria-expanded={rawOpen} onClick={() => setRawOpen(!rawOpen)}>
            <span aria-hidden>{rawOpen ? "▾" : "▸"}</span>
            Raw result
          </button>
          {rawOpen && <pre>{raw}</pre>}
        </div>
      )}
    </div>
  );
}

/** Nodal reviewer's verdict on a work run. */
function VerdictCard({ review, state, onOpenRun }: { review: RunLight; state: RunsState; onOpenRun?: (id: string) => void }) {
  const rv = state.byId.get(review.id);
  return (
    <div className="rd-card">
      <div className="rd-verdict-head">
        <h2 className="section-label">Reviewer verdict</h2>
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
