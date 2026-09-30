import { useState, type ReactNode } from "react";
import { runProgress } from "../../domain/api";
import { projectIdOf, useQueueSummary, useRuns, type RunsState } from "../../domain/hooks/runs";
import { POLL, usePolled } from "../../domain/hooks/store";
import { formatDuration } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useRunActions, type RunActions } from "./actions";
import { executorKindLabel, executorLabel, phaseProgress, runName, runTaskRef, type RunView } from "./status";
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

/** Right-side drawer from the topbar pill: what needs you, what is running, what is next and what failed. */
export function RunMonitor({ onClose, onOpenRun, onGoToRuns }: RunMonitorProps) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const state = useRuns();
  const summary = useQueueSummary();
  const actions = useRunActions();
  const running = runningNow(state);
  const stopped = stoppedRuns(state);
  const waiting = stopped.filter((v) => v.phase === "waiting");
  const failed = stopped.filter((v) => v.phase !== "waiting");
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

          {waiting.length > 0 && (
            <Section title="Waiting on you" count={waiting.length}>
              {waiting.map((v) => (
                <StoppedCard key={v.run.id} v={v} state={state} actions={actions} onOpen={() => onOpenRun(v.run.id)} />
              ))}
            </Section>
          )}

          <Section title="Running" count={running.length}>
            {running.map((v) => (
              <RunningCard key={v.run.id} v={v} state={state} onOpen={() => onOpenRun(v.run.id)} />
            ))}
            {running.length === 0 && <p className="monitor-empty">{state.loaded ? "Nothing running." : "Loading runs…"}</p>}
          </Section>

          <Section title="Up next" count={state.queue.length} hint={state.queue.length > 1 ? "Drag to reorder" : undefined}>
            <UpNext state={state} actions={actions} onOpenRun={onOpenRun} />
          </Section>

          {failed.length > 0 && (
            <Section title="Failed recently" count={failed.length}>
              {failed.map((v) => (
                <StoppedCard key={v.run.id} v={v} state={state} actions={actions} onOpen={() => onOpenRun(v.run.id)} />
              ))}
            </Section>
          )}
        </div>

        <footer className="monitor-foot">
          <span className="monitor-free">
            {cap > 0 &&
              (free > 0
                ? `${free} slot${free === 1 ? "" : "s"} free`
                : `All ${cap} slots busy · next starts when one frees up`)}
          </span>
          <button type="button" className="btn" onClick={onGoToRuns}>
            Go to Runs →
          </button>
        </footer>
      </div>
    </>
  );
}

function Section({ title, count, hint, children }: { title: string; count: number; hint?: string; children: ReactNode }) {
  return (
    <section className="monitor-section" aria-label={title}>
      <h3 className="monitor-section-title">
        {title}
        <span className="monitor-count num">{count}</span>
        {hint && <span className="monitor-hint">{hint}</span>}
      </h3>
      <div className="monitor-list">{children}</div>
    </section>
  );
}

/**
 * One run as a card: title and short status, a two-line "why", a meta line with actions, and a
 * progress hairline. The title's button covers the card; the actions sit above it.
 */
function MonitorCard({
  v,
  state,
  why,
  actions,
  onOpen,
}: {
  v: RunView;
  state: RunsState;
  why: ReactNode;
  actions?: ReactNode;
  onOpen: () => void;
}) {
  const { project, repo, ref } = runRefOf(v, state);
  const pct = v.run.executor.kind === "workflow" ? phaseProgress(v).pct : 0;
  const meta = [ref.key, project?.name, repo?.name, executorLabel(v.run.executor), formatDuration(v.durationMs)]
    .filter((x) => x && x !== "—")
    .join(" · ");
  return (
    <div className={`monitor-card tone-${v.tone}`}>
      <span className="monitor-card-row">
        <button type="button" className="monitor-card-title monitor-open ellipsis" onClick={onOpen}>
          {ref.title}
        </button>
        <span className="monitor-status">
          <span className={`dot dot-sm ${v.pulse ? "pulse" : ""}`} aria-hidden />
          {v.label}
        </span>
      </span>
      {why && <span className="monitor-why">{why}</span>}
      <span className="monitor-card-row monitor-meta-row">
        <span className="monitor-meta ellipsis" title={`${executorKindLabel(v.run.executor)} ${executorLabel(v.run.executor)}`}>
          {meta}
        </span>
        {actions && <span className="monitor-actions">{actions}</span>}
      </span>
      {pct > 0 && <span className="monitor-bar" style={{ width: `${pct}%` }} aria-hidden />}
    </div>
  );
}

function RunningCard({ v, state, onOpen }: { v: RunView; state: RunsState; onOpen: () => void }) {
  const workflow = v.run.executor.kind === "workflow";
  return (
    <MonitorCard
      v={v}
      state={state}
      onOpen={onOpen}
      why={workflow ? <WorkflowProgress v={v} /> : <AgentProgress v={v} />}
    />
  );
}

function WorkflowProgress({ v }: { v: RunView }) {
  if (v.phase !== "running" || v.phaseIndex == null || !v.phaseTotal) return null;
  return <>{`Phase ${v.phaseIndex}/${v.phaseTotal}${v.phaseName ? ` · ${v.phaseName}` : ""}`}</>;
}

function AgentProgress({ v }: { v: RunView }) {
  const live = v.run.status === "launched" && v.run.sessionId != null;
  const q = usePolled<number | null>(
    live ? `run-tool-calls:${v.run.id}` : null,
    () => runProgress(v.run.id).then((p) => p?.toolCalls ?? null),
    [],
    POLL.live,
  );
  const n = q.data;
  if (n == null) return null;
  return <>{`${n} tool call${n === 1 ? "" : "s"}`}</>;
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
    <MonitorCard
      v={v}
      state={state}
      onOpen={onOpen}
      why={<span title={stoppedReason(v)}>{stoppedReason(v)}</span>}
      actions={
        <>
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
        </>
      }
    />
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

  if (queue.length === 0) return <p className="monitor-empty">Queue is empty. New runs start right away while slots are free.</p>;

  return (
    <ol className="monitor-queue">
      {queue.map((v, k) => {
        const { project, repo, ref, name } = runRefOf(v, state);
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
            <span className="queue-grip" aria-hidden>
              ⋮⋮
            </span>
            <span className="queue-pos num">{k + 1}</span>
            <span className="proj-swatch" style={{ background: project?.color ?? "var(--text-disabled)" }} aria-hidden />
            <button type="button" className="queue-open" onClick={() => onOpenRun(v.run.id)}>
              <span className="queue-line">
                {ref.key && <span className="queue-issue">{ref.key}</span>}
                <span className="queue-t ellipsis">{ref.title}</span>
              </span>
              {awaiting ? (
                <span className="queue-await">Migrated · awaiting confirmation</span>
              ) : (
                <span className="queue-sub ellipsis">{[executorLabel(v.run.executor), repo?.name].filter(Boolean).join(" · ")}</span>
              )}
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
