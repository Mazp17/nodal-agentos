import { useState } from "react";
import type { RunView } from "../runs/status";
import { useTaskActions, type TaskActions } from "./actions";
import { NewTaskDialog } from "./NewTaskDialog";
import { TaskCard } from "./TaskCard";
import { TaskPanel } from "./TaskPanel";
import { samePath, useTasks, type TasksState } from "./useTasks";
import type { FinishMode, Task } from "./types";
import "./tasks.css";

export interface TasksViewProps {
  /** Repo a mostrar; `null` = todos los repos. */
  repoPath: string | null;
  /** Repos mapeados (para el selector de "New task"). */
  repos: string[];
  pickFile?: (repoPath: string) => Promise<string | null>;
  /** Abrir el detalle de un run. */
  onOpenRun?: (view: RunView) => void;
  /** Estado compartido (p. ej. el que ya usa el board): evita un segundo polling. Trae todos los repos. */
  state?: TasksState;
  /** Acciones compartidas con `state`. */
  actions?: TaskActions;
  /** "Finish" del repo en Settings, para preseleccionarlo en el drawer. */
  finishOf?: (repoPath: string) => FinishMode | undefined;
}

/** Lista de tareas locales con su diálogo de alta/edición y su drawer de detalle. */
export function TasksView({ repoPath, repos, pickFile, onOpenRun, finishOf, ...shared }: TasksViewProps) {
  const own = useTasks(repoPath, !shared.state);
  const state = shared.state ?? own;
  const ownActions = useTaskActions(state.refresh);
  const actions = shared.actions ?? ownActions;
  const tasks = shared.state && repoPath ? state.tasks.filter((t) => samePath(t.repoPath, repoPath)) : state.tasks;
  const [openId, setOpenId] = useState<string | null>(null);
  const [dialog, setDialog] = useState<{ task: Task | null } | null>(null);
  const [planVersion, setPlanVersion] = useState(0);

  const open = tasks.find((t) => t.id === openId) ?? null;
  const todo = tasks.filter((t) => t.status === "todo");
  const done = tasks.filter((t) => t.status === "done");

  const card = (t: Task) => (
    <TaskCard
      key={t.id}
      task={t}
      view={state.current.get(t.id)}
      showRepo={repoPath === null}
      selected={t.id === openId}
      busy={actions.pending.has(t.id)}
      onOpen={(x) => setOpenId(x.id)}
      onRun={(x) => void actions.run(x)}
      onToggleDone={(x) => void actions.toggleDone(x)}
      onOpenRun={onOpenRun}
    />
  );

  return (
    <div className="tk-view">
      <div className="tk-view-bar">
        <h2 className="tk-view-title">Tasks</h2>
        <span className="tk-count num">{tasks.length}</span>
        <button type="button" className="btn btn-primary btn-sm tk-view-new" onClick={() => setDialog({ task: null })}>
          New task
        </button>
      </div>
      {state.error && (
        <div className="banner banner-error" role="alert">
          {state.error}
        </div>
      )}
      <div className="tk-view-body">
        {!state.loading && tasks.length === 0 ? (
          <div className="center-state">
            <div className="center-state-body">
              <span className="state-icon-empty" aria-hidden />
              <span className="center-state-title">No tasks yet</span>
              <span className="center-state-text">
                Hand a plan to a repository and let /plan-task carry it to a PR — no Linear issue needed.
              </span>
              <div className="center-state-actions">
                <button type="button" className="btn btn-primary" onClick={() => setDialog({ task: null })}>
                  New task
                </button>
              </div>
            </div>
          </div>
        ) : (
          <>
            <section className="tk-group" aria-label="To do">
              <h3 className="section-label">To do · {todo.length}</h3>
              <div className="tk-grid">{todo.map(card)}</div>
            </section>
            {done.length > 0 && (
              <section className="tk-group" aria-label="Done">
                <h3 className="section-label">Done · {done.length}</h3>
                <div className="tk-grid">{done.map(card)}</div>
              </section>
            )}
          </>
        )}
      </div>

      {open && (
        <TaskPanel
          task={open}
          current={state.current.get(open.id)}
          history={state.historyOf(open.id)}
          actions={actions}
          planVersion={planVersion}
          defaultFinish={finishOf?.(open.repoPath)}
          onClose={() => setOpenId(null)}
          onEdit={(t) => setDialog({ task: t })}
          onOpenRun={onOpenRun}
        />
      )}
      {dialog && (
        <NewTaskDialog
          repos={repos}
          defaultRepo={repoPath}
          task={dialog.task}
          pickFile={pickFile}
          onClose={() => setDialog(null)}
          onSaved={(t) => {
            setDialog(null);
            setPlanVersion((v) => v + 1);
            setOpenId(t.id);
            void state.refresh();
          }}
        />
      )}
    </div>
  );
}
