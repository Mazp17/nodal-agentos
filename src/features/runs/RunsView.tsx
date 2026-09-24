import { useState } from "react";
import { projectIdOf, useQueueSummary, useRuns } from "../../domain/hooks/runs";
import { formatDuration, formatTokens } from "../../lib/format";
import { useRunActions } from "./actions";
import { executorLabel, phaseProgress, runName, runTaskRef, type RunsTab, type RunView } from "./status";
import "./runs.css";

const TABS: { id: RunsTab; label: string }[] = [
  { id: "active", label: "Active" },
  { id: "queued", label: "Queued" },
  { id: "finished", label: "Finished" },
  { id: "failed", label: "Failed" },
];

const EMPTY: Record<RunsTab, string> = {
  active: "Nothing running. Run a task from the board.",
  queued: "Nothing queued.",
  finished: "No finished runs yet.",
  failed: "No failed runs.",
};

export interface RunsViewProps {
  /** `null`: todos los proyectos. La cola siempre es global. */
  projectId: string | null;
  onOpenRun: (runId: string) => void;
}

/** Pantalla "Runs": tabs por estado, tabla de runs y la cola global con sus slots. */
export function RunsView({ projectId, onOpenRun }: RunsViewProps) {
  const state = useRuns({ projectId });
  const summary = useQueueSummary();
  const [tab, setTab] = useState<RunsTab>("active");
  const counts = Object.fromEntries(TABS.map((t) => [t.id, state.views.filter((v) => v.tab === t.id).length])) as Record<
    RunsTab,
    number
  >;
  const rows = state.views.filter((v) => v.tab === tab);
  const scope = projectId ? (state.projects.get(projectId)?.name ?? "") : "All projects";

  return (
    <div className="runs">
      <div className="runs-main">
        <div className="runs-tabs">
          <div className="runs-tablist" role="tablist" aria-label="Runs">
            {TABS.map((t) => (
              <button
                key={t.id}
                id={`runs-tab-${t.id}`}
                type="button"
                role="tab"
                aria-selected={tab === t.id}
                className={`runs-tab ${tab === t.id ? "on" : ""}`}
                onClick={() => setTab(t.id)}
              >
                {t.label}
                <span className="runs-tab-count num">{counts[t.id]}</span>
              </button>
            ))}
          </div>
          <span className="runs-scope">{scope}</span>
        </div>
        {state.error && <div className="banner banner-error runs-error">{state.error}</div>}
        {state.liveError && <div className="banner banner-warn runs-error">{state.liveError}</div>}
        <div className="table-head runs-cols">
          <span>Project · repo</span>
          <span>Task</span>
          <span>Executor</span>
          <span>Phase</span>
          <span className="text-right">Time</span>
          <span className="text-right">Tokens</span>
          <span>Result</span>
        </div>
        <div className="runs-rows" role="tabpanel" aria-labelledby={`runs-tab-${tab}`}>
          {rows.map((v) => (
            <RunRow key={v.run.id} v={v} state={state} onOpen={() => onOpenRun(v.run.id)} />
          ))}
          {rows.length === 0 && <div className="runs-empty">{state.loaded ? EMPTY[tab] : "Loading runs…"}</div>}
        </div>
      </div>
      <QueuePanel state={state} summary={summary} onOpenRun={onOpenRun} />
    </div>
  );
}

function RunRow({ v, state, onOpen }: { v: RunView; state: ReturnType<typeof useRuns>; onOpen: () => void }) {
  const task = v.run.taskId ? state.tasks.get(v.run.taskId) : undefined;
  const pid = projectIdOf(v.run, state.tasks, state.repos);
  const project = pid ? state.projects.get(pid) : undefined;
  const repo = v.run.repoId ? state.repos.get(v.run.repoId) : task ? state.repos.get(task.repoId) : undefined;
  const ref = runTaskRef(v.run, task, project);
  const ph = phaseProgress(v);
  const queued = v.tab === "queued";
  return (
    <button type="button" className="runs-cols runs-row" onClick={onOpen}>
      <span className="runs-proj">
        <span className="proj-swatch" style={{ background: project?.color ?? "var(--text-disabled)" }} aria-hidden />
        <span className="ellipsis">
          {project?.name ?? "—"} <span className="runs-repo">· {repo?.name ?? "—"}</span>
        </span>
      </span>
      <span className="runs-task">
        {ref.key && <span className="runs-key">{ref.key}</span>}
        <span className="ellipsis">{ref.title}</span>
      </span>
      <span className="runs-exec ellipsis" title={v.run.kind === "review" ? "Reviewer" : undefined}>
        {v.run.kind === "review" && <span className="runs-kind">Review</span>}
        {executorLabel(v.run.executor)}
      </span>
      <span className={`runs-phase tone-${v.tone}`}>
        <span className="runs-phase-text ellipsis">{ph.text}</span>
        <span className="runs-bar">
          <span style={{ width: `${ph.pct}%` }} />
        </span>
      </span>
      <span className="text-right num runs-num">{queued ? "—" : formatDuration(v.durationMs)}</span>
      <span className="text-right num runs-num">{formatTokens(v.tokens)}</span>
      <span className={`runs-result tone-${v.tone}`}>
        <span className={`dot dot-sm ${v.pulse ? "pulse" : ""}`} aria-hidden />
        <span className="ellipsis">{v.phase === "running" ? "Running" : v.label}</span>
      </span>
    </button>
  );
}

function QueuePanel({
  state,
  summary,
  onOpenRun,
}: {
  state: ReturnType<typeof useRuns>;
  summary: ReturnType<typeof useQueueSummary>;
  onOpenRun: (id: string) => void;
}) {
  const actions = useRunActions();
  const queue = state.queue;
  const cap = summary.capacity;
  const slots = Math.min(Math.max(cap, summary.running), 16);
  const full = cap > 0 && summary.running >= cap;
  const ids = queue.map((v) => v.run.id);

  const moveUp = (k: number) => {
    if (k <= 0) return;
    const next = [...ids];
    [next[k - 1], next[k]] = [next[k], next[k - 1]];
    void actions.reorder(next);
  };

  return (
    <aside className="queue" aria-labelledby="queue-title">
      <div className="queue-head">
        <h2 id="queue-title" className="queue-title">
          Global queue
        </h2>
        <span className="queue-cap">
          {summary.running} of {cap || "?"} slots in use
        </span>
      </div>
      {slots > 0 && (
        <div className="queue-slots" style={{ gridTemplateColumns: `repeat(${slots}, 1fr)` }} aria-hidden>
          {Array.from({ length: slots }, (_, k) => (
            <span key={k} className={k < summary.running ? "on" : ""} />
          ))}
        </div>
      )}
      {full && (
        <div className="queue-full">
          All {cap} slots are busy. {queue.length ? `${queue.length} waiting; the` : "The"} next run starts when one finishes.
        </div>
      )}
      {queue.map((v, k) => {
        const task = v.run.taskId ? state.tasks.get(v.run.taskId) : undefined;
        const pid = projectIdOf(v.run, state.tasks, state.repos);
        const project = pid ? state.projects.get(pid) : undefined;
        const ref = runTaskRef(v.run, task, project);
        const name = runName(v.run, task, project);
        const awaiting = v.phase === "awaiting";
        return (
          <div key={v.run.id} className={`queue-item ${awaiting ? "queue-item-await" : ""}`}>
            <span className="queue-pos num">{k + 1}</span>
            <span className="proj-swatch" style={{ background: project?.color ?? "var(--text-disabled)" }} aria-hidden />
            <button type="button" className="queue-open" onClick={() => onOpenRun(v.run.id)}>
              <span className="queue-issue">{ref.key ?? executorLabel(v.run.executor)}</span>
              <span className="queue-t ellipsis">{ref.title}</span>
              {awaiting && <span className="queue-await">Migrated · awaiting confirmation</span>}
            </button>
            {awaiting && (
              <button
                type="button"
                className="btn btn-xs btn-amber"
                disabled={actions.busy}
                onClick={() => void actions.confirm(v, name)}
              >
                Confirm
              </button>
            )}
            <button
              type="button"
              className="icon-btn queue-btn"
              aria-label={`Move ${name} up`}
              disabled={actions.busy || k === 0}
              onClick={() => moveUp(k)}
            >
              ▲
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
          </div>
        );
      })}
      {queue.length === 0 && <p className="queue-empty">Queue is empty. New runs start immediately while slots are free.</p>}
    </aside>
  );
}
