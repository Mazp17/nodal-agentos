import { useState, type ReactNode } from "react";
import { getRunTranscript } from "../../domain/api";
import { projectIdOf, useQueueSummary, useRuns, type RunsState } from "../../domain/hooks/runs";
import { POLL, usePolled } from "../../domain/hooks/store";
import { formatDuration } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useRunActions, type RunActions } from "./actions";
import { PhaseSegments } from "./RunBadge";
import { executorKindLabel, executorLabel, runName, runTaskRef, type RunView } from "./status";
import "./runs.css";
import "./run-monitor.css";

export interface RunMonitorProps {
  onClose: () => void;
  onOpenRun: (runId: string) => void;
  onGoToRuns: () => void;
}

const isLaunched = (v: RunView) => v.run.status === "launching" || v.run.status === "launched";

/** Launched runs that aren't waiting on the user. */
function runningNow(state: RunsState): RunView[] {
  return state.views.filter((v) => isLaunched(v) && v.phase !== "waiting");
}

/** Sessions waiting on the user, then ended runs whose task is still Blocked. */
function stoppedRuns(state: RunsState): RunView[] {
  const waiting = state.views.filter((v) => isLaunched(v) && v.phase === "waiting");
  const failed = [...state.latestByTask.values()].filter((v) => {
    const task = v.run.taskId ? state.tasks.get(v.run.taskId) : undefined;
    // Runs that die mid-session end as `finished` (outcome red/unknown), not `failed`.
    const ended = v.phase === "failed" || v.phase === "finished" || (v.phase === "canceled" && v.run.outcome === "stopped");
    return ended && task?.status === "blocked";
  });
  return [...waiting, ...failed];
}

/** `ids` with the item at `from` moved to index `to`. */
function moveId(ids: string[], from: number, to: number): string[] {
  const next = [...ids];
  const [id] = next.splice(from, 1);
  next.splice(Math.max(0, Math.min(to, next.length)), 0, id);
  return next;
}

function runRefOf(v: RunView, state: RunsState) {
  const task = v.run.taskId ? state.tasks.get(v.run.taskId) : undefined;
  const pid = projectIdOf(v.run, state.tasks, state.repos);
  const project = pid ? state.projects.get(pid) : undefined;
  const repoId = v.run.repoId ?? task?.repoId;
  const repo = repoId ? state.repos.get(repoId) : undefined;
  return { task, project, repo, ref: runTaskRef(v.run, task, project), name: runName(v.run, task, project) };
}

/** Right-side drawer from the topbar pill: what is running, what stopped and what is next. */
export function RunMonitor({ onClose, onOpenRun, onGoToRuns }: RunMonitorProps) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const state = useRuns();
  const summary = useQueueSummary();
  const actions = useRunActions();
  const running = runningNow(state);
  const stopped = stoppedRuns(state);
  const cap = summary.capacity;
  const slots = Math.min(Math.max(cap, summary.running), 16);
  const free = Math.max(0, cap - summary.running);

  return (
    <>
      <div className="scrim monitor-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="sheet monitor" role="dialog" aria-modal="true" aria-labelledby="monitor-title" tabIndex={-1}>
        <header className="monitor-head">
          <div className="monitor-head-row">
            <h2 id="monitor-title" className="monitor-title">
              Run monitor
            </h2>
            <span className="monitor-cap num">
              {summary.running}/{cap || "?"} slots
            </span>
            <button type="button" className="icon-btn monitor-close" aria-label="Close run monitor" onClick={onClose}>
              ✕
            </button>
          </div>
          {slots > 0 && (
            <div className="queue-slots" style={{ gridTemplateColumns: `repeat(${slots}, 1fr)` }} aria-hidden>
              {Array.from({ length: slots }, (_, k) => (
                <span key={k} className={k < summary.running ? "on" : ""} />
              ))}
            </div>
          )}
        </header>

        <div className="monitor-body">
          {state.error && <div className="banner banner-error monitor-error">{state.error}</div>}

          <Section title="Running now" count={running.length}>
            {running.map((v) => (
              <RunningCard key={v.run.id} v={v} state={state} onOpen={() => onOpenRun(v.run.id)} />
            ))}
            {running.length === 0 && <p className="monitor-empty">{state.loaded ? "Nothing running." : "Loading runs…"}</p>}
          </Section>

          {stopped.length > 0 && (
            <Section title="Stopped" count={stopped.length}>
              {stopped.map((v) => (
                <StoppedCard key={v.run.id} v={v} state={state} actions={actions} onOpen={() => onOpenRun(v.run.id)} />
              ))}
            </Section>
          )}

          <Section title="Up next" count={state.queue.length}>
            <UpNext state={state} actions={actions} onOpenRun={onOpenRun} />
          </Section>
        </div>

        <footer className="monitor-foot">
          <span className="monitor-free">
            {cap > 0 &&
              (free > 0
                ? `${free} slot${free === 1 ? "" : "s"} free`
                : `All ${cap} slots busy · next starts when one frees up`)}
          </span>
          <button type="button" className="btn btn-sm" onClick={onGoToRuns}>
            Go to Runs
          </button>
        </footer>
      </div>
    </>
  );
}

function Section({ title, count, children }: { title: string; count: number; children: ReactNode }) {
  return (
    <section className="monitor-section" aria-label={title}>
      <h3 className="monitor-section-title">
        {title}
        <span className="monitor-count num">{count}</span>
      </h3>
      {children}
    </section>
  );
}

function RunTitle({ v, state }: { v: RunView; state: RunsState }) {
  const { project, ref } = runRefOf(v, state);
  return (
    <span className="monitor-run-title">
      <span className="proj-swatch" style={{ background: project?.color ?? "var(--text-disabled)" }} aria-hidden />
      {ref.key && <span className="monitor-key">{ref.key}</span>}
      <span className="ellipsis">{ref.title}</span>
    </span>
  );
}

function RunningCard({ v, state, onOpen }: { v: RunView; state: RunsState; onOpen: () => void }) {
  const { repo } = runRefOf(v, state);
  const workflow = v.run.executor.kind === "workflow";
  return (
    <button type="button" className={`monitor-card tone-${v.tone}`} onClick={onOpen}>
      <span className="monitor-card-row">
        <RunTitle v={v} state={state} />
        <span className="monitor-time num">{formatDuration(v.durationMs)}</span>
      </span>
      <span className="monitor-card-row monitor-meta">
        <span className="monitor-exec ellipsis">
          <span className="monitor-kind">{executorKindLabel(v.run.executor)}</span>
          {executorLabel(v.run.executor)}
        </span>
        <span className="monitor-progress ellipsis">
          <span className={`dot dot-sm ${v.pulse ? "pulse" : ""}`} aria-hidden />
          {workflow ? <WorkflowProgress v={v} /> : <AgentProgress v={v} repo={repo?.name ?? null} />}
        </span>
      </span>
      {workflow && <PhaseSegments run={v} />}
    </button>
  );
}

function WorkflowProgress({ v }: { v: RunView }) {
  if (v.phase !== "running" || v.phaseIndex == null || !v.phaseTotal) return <>{v.label}</>;
  return <>{`Phase ${v.phaseIndex}/${v.phaseTotal}${v.phaseName ? ` · ${v.phaseName}` : ""}`}</>;
}

function AgentProgress({ v, repo }: { v: RunView; repo: string | null }) {
  const live = v.run.status === "launched" && v.run.sessionId != null;
  const q = usePolled<number | null>(
    live ? `run-tool-calls:${v.run.id}` : null,
    () => getRunTranscript(v.run.id, 1).then((t) => t?.toolCalls ?? null),
    [],
    POLL.live,
  );
  const n = q.data;
  if (n == null) return <>{repo ? `${v.label} · ${repo}` : v.label}</>;
  return <>{`${n} tool call${n === 1 ? "" : "s"}${repo ? ` · ${repo}` : ""}`}</>;
}

function stoppedReason(v: RunView): string {
  if (v.phase === "waiting") {
    return v.waitingFor && v.waitingFor !== "input needed"
      ? `The session is waiting on you: ${v.waitingFor}.`
      : "The session is waiting on you.";
  }
  return v.run.error ?? `${v.label}. The task is blocked.`;
}

function StoppedCard({ v, state, actions, onOpen }: { v: RunView; state: RunsState; actions: RunActions; onOpen: () => void }) {
  const { task, name } = runRefOf(v, state);
  const waiting = v.phase === "waiting";
  const retryOf = v.run.kind === "review" && v.run.parentRunId ? (state.byId.get(v.run.parentRunId) ?? v) : v;
  const canRetry = !waiting && v.run.taskId != null;
  return (
    <div className={`monitor-card monitor-stopped tone-${v.tone}`}>
      <span className="monitor-card-row">
        <RunTitle v={v} state={state} />
        <span className={`badge badge-sm tone-${v.tone}`}>{v.label}</span>
      </span>
      <p className="monitor-reason" title={stoppedReason(v)}>
        {stoppedReason(v)}
      </p>
      <span className="monitor-actions">
        <button type="button" className="btn btn-xs" onClick={onOpen}>
          Open run
        </button>
        {waiting && (
          <button type="button" className="btn btn-xs btn-primary" disabled={actions.busy} onClick={() => void actions.attach(v)}>
            Attach
          </button>
        )}
        {canRetry && (
          <button
            type="button"
            className="btn btn-xs btn-primary"
            disabled={actions.busy}
            onClick={() => void actions.runAgain(retryOf, task, name)}
          >
            Retry
          </button>
        )}
      </span>
    </div>
  );
}

function UpNext({ state, actions, onOpenRun }: { state: RunsState; actions: RunActions; onOpenRun: (id: string) => void }) {
  const queue = state.queue;
  const ids = queue.map((v) => v.run.id);
  const [drag, setDrag] = useState<{ from: number; over: number } | null>(null);

  const move = (from: number, to: number) => {
    if (to < 0 || to >= ids.length || to === from) return;
    void actions.reorder(moveId(ids, from, to));
  };

  if (queue.length === 0) return <p className="monitor-empty">Queue is empty. New runs start immediately while slots are free.</p>;

  return (
    <ol className="monitor-queue">
      {queue.map((v, k) => {
        const { project, ref, name } = runRefOf(v, state);
        const awaiting = v.phase === "awaiting";
        const over = drag && drag.over === k && drag.from !== k ? (drag.from < k ? "below" : "above") : "";
        return (
          <li
            key={v.run.id}
            className={`queue-item monitor-queue-item ${awaiting ? "queue-item-await" : ""} ${drag?.from === k ? "dragging" : ""} ${over ? `drop-${over}` : ""}`}
            draggable={!actions.busy}
            onDragStart={(e) => {
              e.dataTransfer.effectAllowed = "move";
              e.dataTransfer.setData("text/plain", v.run.id);
              setDrag({ from: k, over: k });
            }}
            onDragOver={(e) => {
              if (!drag) return;
              e.preventDefault();
              e.dataTransfer.dropEffect = "move";
              if (drag.over !== k) setDrag({ ...drag, over: k });
            }}
            onDrop={(e) => {
              e.preventDefault();
              if (drag) move(drag.from, k);
              setDrag(null);
            }}
            onDragEnd={() => setDrag(null)}
          >
            <span className="queue-pos num">{k + 1}</span>
            <span className="proj-swatch" style={{ background: project?.color ?? "var(--text-disabled)" }} aria-hidden />
            <button type="button" className="queue-open" onClick={() => onOpenRun(v.run.id)}>
              <span className="queue-issue">{ref.key ?? executorLabel(v.run.executor)}</span>
              <span className="queue-t ellipsis">{ref.title}</span>
              {awaiting && <span className="queue-await">Migrated · awaiting confirmation</span>}
            </button>
            {awaiting && (
              <button type="button" className="btn btn-xs btn-amber" disabled={actions.busy} onClick={() => void actions.confirm(v, name)}>
                Confirm
              </button>
            )}
            <button
              type="button"
              className="icon-btn queue-btn"
              aria-label={`Move ${name} up`}
              disabled={actions.busy || k === 0}
              onClick={() => move(k, k - 1)}
            >
              ▲
            </button>
            <button
              type="button"
              className="icon-btn queue-btn"
              aria-label={`Move ${name} down`}
              disabled={actions.busy || k === queue.length - 1}
              onClick={() => move(k, k + 1)}
            >
              ▼
            </button>
            <button
              type="button"
              className="icon-btn queue-btn"
              aria-label={`Remove ${name} from queue`}
              disabled={actions.busy}
              onClick={() => void actions.remove(v, name)}
            >
              ✕
            </button>
          </li>
        );
      })}
    </ol>
  );
}
