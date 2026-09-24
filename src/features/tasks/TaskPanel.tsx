import { useEffect, useId, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  addTaskRelation,
  cancelRun,
  cleanupWorktree,
  deleteTask,
  removeTaskRelation,
  unlinkTask,
  updateTask,
  type TaskPatch,
} from "../../domain/api";
import {
  invalidate,
  isRunActive,
  useAllRuns,
  useProjects,
  useRepos,
  useSettings,
  useTask,
  useTaskPlan,
  useTaskRelations,
  useTaskRuns,
  useTasks,
} from "../../domain/hooks/tasks";
import { taskKey, TASK_STATUSES, type Executor, type RelationKind, type Run, type Task, type TaskStatus } from "../../domain/types";
import { formatDateTime, formatDuration } from "../../lib/format";
import { SafeMarkdown } from "../../ui/Markdown";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useToast } from "../../ui/Toasts";
import { ExecutorName, ExecutorPicker, executorLabel, resolveExecutor } from "../executors";
import { SourceTab } from "../providers";
import { RunBadge } from "../runs";
import { NewTaskDialog } from "./NewTaskDialog";
import { PriorityBars, StatusRing } from "./bits";
import { FINISH_LABEL, isClosed, ISOLATION_LABEL, PRIORITY_LABEL, providerLabel, STATUS_META } from "./status";
import { useLaunch } from "./useLaunch";
import "./tasks.css";

export interface TaskPanelProps {
  taskId: string;
  onClose: () => void;
  onOpenRun: (runId: string) => void;
  /** Abre el diff de un run (RunDiffDrawer). */
  onOpenDiff: (runId: string) => void;
  /** Navegar a una tarea relacionada; sin él, las relaciones no son clickeables. */
  onOpenTask?: (taskId: string) => void;
}

function useOutside(open: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open, close]);
  return ref;
}

/** Detalle de tarea (drawer): estado, repo, plan, criterios, relaciones, cadena de pasos y lanzamiento. */
export function TaskPanel({ taskId, onClose, onOpenRun, onOpenDiff, onOpenTask }: TaskPanelProps) {
  const ref = useFocusTrap<HTMLElement>(onClose);
  const headingId = useId();
  const push = useToast();
  const launch = useLaunch();

  const taskQ = useTask(taskId);
  const task = taskQ.data;
  const projects = useProjects();
  const repos = useRepos(task?.projectId ?? null);
  const runsQ = useTaskRuns(taskId);
  const allRuns = useAllRuns();
  const settings = useSettings();

  const [tab, setTab] = useState<"overview" | "source">("overview");
  const [menu, setMenu] = useState<"status" | "repo" | null>(null);
  const [editing, setEditing] = useState(false);
  const [launchExec, setLaunchExec] = useState<Executor | null>(null);
  const [extra, setExtra] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  useEffect(() => {
    setTab("overview");
    setLaunchExec(null);
    setExtra("");
    setMenu(null);
  }, [taskId]);

  const closeMenu = () => setMenu(null);
  const statusRef = useOutside(menu === "status", closeMenu);
  const repoRef = useOutside(menu === "repo", closeMenu);

  const project = projects.data?.find((p) => p.id === task?.projectId);
  const projectRepos = (repos.data ?? []).filter((r) => r.projectId === task?.projectId);
  const repo = projectRepos.find((r) => r.id === task?.repoId);
  const runs = runsQ.data ?? [];
  const chain = useMemo(() => [...(runsQ.data ?? [])].reverse(), [runsQ.data]);
  const current = runs[0];
  const activeRun = runs.find(isRunActive);
  const workRuns = runs.filter((r) => r.kind === "work" && r.launchedAt != null);
  const lastWork = workRuns[0];

  if (!task) {
    return (
      <Shell refEl={ref} headingId={headingId} onClose={onClose}>
        <div className="tp-body">
          {taskQ.error ? (
            <div className="banner banner-error" role="alert">
              {taskQ.error}
            </div>
          ) : (
            <span className="tk-muted">Loading task…</span>
          )}
        </div>
      </Shell>
    );
  }

  const key = project ? taskKey(project.key, task.number) : `#${task.number}`;
  const assignee = resolveExecutor(task, repo, project);
  const runExec = launchExec ?? assignee;
  const src = task.source;
  const closed = isClosed(task.status);
  const canLaunch = !!repo && !activeRun && !closed;
  const slotsBusy = (allRuns.data ?? []).filter((r) => r.status === "launching" || r.status === "launched").length;
  const queued = (allRuns.data ?? []).some((r) => r.status === "queued");
  const willQueue = queued || (settings.data ? slotsBusy >= settings.data.concurrency : false);

  const act = async (label: string, fn: () => Promise<unknown>, okMsg?: [string, string?]) => {
    setBusy(label);
    try {
      await fn();
      if (okMsg) push(okMsg[0], okMsg[1], "ok");
    } catch (e) {
      push(`Couldn't ${label}`, String(e), "danger");
    } finally {
      setBusy(null);
      invalidate("tasks", "runs");
    }
  };

  const patch = (p: TaskPatch, label: string, ok?: [string, string?]) => act(label, () => updateTask(task.id, p), ok);

  const setStatus = (s: TaskStatus) => {
    setMenu(null);
    if (s !== task.status) void patch({ status: s }, "change the status", [`${key} → ${STATUS_META[s].label}`, task.title]);
  };

  const doLaunch = async (how: "run" | "handoff" | "review") => {
    setBusy(how);
    const extraText = extra.trim() || null;
    const run =
      how === "run"
        ? await launch(task.id, key, {
            kind: "run",
            input: { ...(launchExec ? { executor: launchExec } : {}), ...(extraText ? { extraInstructions: extraText } : {}) },
          })
        : how === "handoff"
          ? await launch(task.id, key, { kind: "handoff", executor: runExec, extra: extraText })
          : await launch(task.id, key, { kind: "review" });
    setBusy(null);
    if (run) {
      setExtra("");
      setLaunchExec(null);
    }
  };

  const cleanUp = () => {
    const wt = task.worktree;
    if (!wt) return;
    const ok = window.confirm(
      `Delete the worktree and the branch ${wt.branch}?\n\n${wt.path}\n\nUncommitted changes and commits that were not pushed or merged are lost. This can't be undone.`,
    );
    if (ok) void act("clean up the worktree", () => cleanupWorktree(task.id), ["Worktree cleaned up", wt.branch]);
  };

  const remove = () => {
    if (!window.confirm(`Delete ${key} "${task.title}"? This can't be undone.`)) return;
    void act("delete the task", async () => {
      await deleteTask(task.id);
      onClose();
    }, ["Task deleted", task.title]);
  };

  const unlink = () => {
    if (!src) return;
    if (!window.confirm(`Unlink ${src.identifier}? It stays in ${providerLabel(src.provider)}; this task becomes local and stops syncing.`)) return;
    void act("unlink the task", () => unlinkTask(task.id), [`${key} unlinked`, `${src.identifier} stays in ${providerLabel(src.provider)}.`]);
  };

  return (
    <Shell refEl={ref} headingId={headingId} onClose={onClose}>
      <div className="tp-head">
        <div className="tp-head-row">
          <div className="tp-menu-wrap" ref={statusRef}>
            <button
              type="button"
              className="tp-status"
              aria-haspopup="menu"
              aria-expanded={menu === "status"}
              aria-label={`Status: ${STATUS_META[task.status].label}`}
              onClick={() => setMenu(menu === "status" ? null : "status")}
            >
              <StatusRing status={task.status} />
              {STATUS_META[task.status].label}
              <span className="ex-caret" aria-hidden>▼</span>
            </button>
            {menu === "status" && (
              <MenuList
                label="Status"
                onEscape={closeMenu}
                items={TASK_STATUSES.map((s) => ({
                  key: s,
                  label: STATUS_META[s].label,
                  dot: STATUS_META[s].color,
                  checked: s === task.status,
                  onPick: () => setStatus(s),
                }))}
              />
            )}
          </div>
          <span className="mono tp-key">{key}</span>
          <PriorityBars priority={task.priority} />
          <span className="tp-spacer" />
          <button type="button" className="icon-btn" aria-label="Close task" onClick={onClose}>
            ✕
          </button>
        </div>
        <h2 id={headingId} className="tp-title">
          {task.title}
        </h2>
        <div className="tp-meta">
          {project && (
            <span className="tp-chip">
              <span className="dlg-proj-dot" style={{ background: project.color }} aria-hidden />
              {project.name}
            </span>
          )}
          <div className="tp-menu-wrap" ref={repoRef}>
            <button
              type="button"
              className={`tp-chip tp-chip-btn ${repo ? "" : "tp-danger"}`}
              aria-haspopup="menu"
              aria-expanded={menu === "repo"}
              aria-label={`Repo: ${repo?.name ?? "removed"}. Move to another repo`}
              title={repo?.path}
              onClick={() => setMenu(menu === "repo" ? null : "repo")}
            >
              {repo ? repo.name : "Repo removed"} ▾
            </button>
            {menu === "repo" && (
              <MenuList
                label="Move to repo"
                onEscape={closeMenu}
                items={projectRepos.map((r) => ({
                  key: r.id,
                  label: r.name,
                  checked: r.id === task.repoId,
                  onPick: () => {
                    setMenu(null);
                    if (r.id !== task.repoId) void patch({ repoId: r.id }, "move the task", [`${key} moved`, `Now runs in ${r.name}`]);
                  },
                }))}
              />
            )}
          </div>
          {src ? (
            <span className="tp-chip">
              <button type="button" className="tp-link mono" onClick={() => void openUrl(src.url).catch(() => {})} title={src.url}>
                {src.identifier} ↗
              </button>
              {src.externalState && <span className="tk-muted">{src.externalState.name}</span>}
              <span className={`dot dot-sm ${src.syncError ? "tone-danger" : "tone-muted"}`} aria-hidden />
              <span className="tk-muted">
                {src.syncError ? "Sync failed" : src.lastSyncedAt ? `Synced ${formatDateTime(src.lastSyncedAt)}` : "Not synced yet"}
              </span>
            </span>
          ) : (
            <span className="tp-chip tp-local">Local</span>
          )}
          <span className="tp-chip" title="Assignee">
            <ExecutorName executor={assignee} />
          </span>
        </div>
        {src?.syncError && (
          <div className="banner banner-error tp-banner" role="alert">
            {providerLabel(src.provider)} sync failed: {src.syncError}
          </div>
        )}
        {!repo && repos.data && (
          <div className="banner banner-warn tp-banner">
            This task's repo was removed from the project. Choose another repo to run it.
          </div>
        )}
        {src && (
          <div className="segmented tp-tabs" role="tablist" aria-label="Task sections">
            {(
              [
                ["overview", "Overview"],
                ["source", providerLabel(src.provider)],
              ] as const
            ).map(([k, l]) => (
              <button
                key={k}
                type="button"
                role="tab"
                aria-selected={tab === k}
                className={`tk-seg-opt ${tab === k ? "on" : ""}`}
                onClick={() => setTab(k)}
              >
                {l}
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="tp-body">
        {tab === "source" && src ? (
          <SourceTab task={task} />
        ) : (
          <>
            <PlanSection task={task} />
            <AcceptanceSection task={task} onEdit={() => setEditing(true)} />
            <RelationsSection task={task} onOpenTask={onOpenTask} />

            <section className="tp-section" aria-label="Steps">
              <span className="section-label">Steps</span>
              {chain.length === 0 ? (
                <span className="tk-muted">No runs yet.</span>
              ) : (
                <ol className="tp-chain">
                  {chain.map((r, i) => (
                    <StepRow key={r.id} run={r} n={i + 1} onOpenRun={onOpenRun} onOpenDiff={onOpenDiff} />
                  ))}
                </ol>
              )}
            </section>

            {task.worktree && (
              <section className="tp-section" aria-label="Worktree">
                <span className="section-label">Worktree</span>
                <div className="tp-wt">
                  <div className="tp-wt-row">
                    <span className="tk-muted">Branch</span>
                    <span className="mono ellipsis">{task.worktree.branch}</span>
                    <span className="tk-muted">from</span>
                    <span className="mono">{task.worktree.base}</span>
                  </div>
                  <div className="tp-wt-row">
                    <span className="tk-muted">Path</span>
                    <span className="mono ellipsis" title={task.worktree.path}>
                      {task.worktree.path}
                    </span>
                  </div>
                  <div className="tp-wt-actions">
                    {lastWork && (
                      <button type="button" className="btn btn-sm" onClick={() => onOpenDiff(lastWork.id)}>
                        View diff
                      </button>
                    )}
                    <button
                      type="button"
                      className="btn btn-sm btn-danger"
                      disabled={!!activeRun || busy !== null}
                      title={activeRun ? "Cancel the running step first" : undefined}
                      onClick={cleanUp}
                    >
                      Clean up
                    </button>
                  </div>
                </div>
              </section>
            )}

            {activeRun && (
              <section className="tp-section tp-launch" aria-label="Current run">
                <span className="section-label">{activeRun.status === "queued" ? "Queued" : "Running"}</span>
                <div className="tp-running">
                  <ExecutorName executor={activeRun.executor} />
                  <RunBadge run={activeRun} />
                  <span className="tp-spacer" />
                  <button type="button" className="btn btn-sm" onClick={() => onOpenRun(activeRun.id)}>
                    Open run
                  </button>
                  <button
                    type="button"
                    className="btn btn-sm btn-danger"
                    disabled={busy !== null}
                    onClick={() =>
                      void act(activeRun.status === "queued" ? "cancel the run" : "stop the run", () => cancelRun(activeRun.id), [
                        activeRun.status === "queued" ? "Removed from the queue" : "Run stopped",
                        key,
                      ])
                    }
                  >
                    {activeRun.status === "queued" ? "Cancel" : "Stop"}
                  </button>
                </div>
              </section>
            )}

            {canLaunch && (
              <section className="tp-section tp-launch" aria-label="Launch">
                <span className="section-label">Launch</span>
                <div className="tp-launch-row">
                  <span className="tk-muted">Executor for this run</span>
                  <ExecutorPicker
                    repoId={repo?.id ?? null}
                    value={launchExec}
                    inherited={assignee}
                    label="Executor for this run"
                    onChange={setLaunchExec}
                    dropUp
                  />
                </div>
                <textarea
                  className="textarea tp-extra"
                  rows={2}
                  value={extra}
                  onChange={(e) => setExtra(e.target.value)}
                  placeholder="Extra instructions for this run (optional)"
                  aria-label="Extra instructions for this run"
                />
                <div className="tp-launch-actions">
                  <span className="tk-hint">
                    {[
                      runExec.kind === "workflow" ? null : ISOLATION_LABEL[task.isolation ?? repo?.defaultIsolation ?? "worktree"],
                      FINISH_LABEL[task.finish ?? repo?.defaultFinish ?? "pr"],
                      (task.review ?? repo?.defaultReview ?? true) ? "review on" : "review off",
                    ]
                      .filter(Boolean)
                      .join(" · ")}
                  </span>
                  <span className="tp-spacer" />
                  {lastWork && (
                    <button type="button" className="btn btn-sm" disabled={busy !== null} onClick={() => void doLaunch("review")}>
                      Review now
                    </button>
                  )}
                  {lastWork && (
                    <button
                      type="button"
                      className="btn btn-sm"
                      disabled={busy !== null}
                      title="Next step on the same branch: gets the plan, criteria, previous summary and diff"
                      onClick={() => void doLaunch("handoff")}
                    >
                      Hand off → {executorLabel(runExec)}
                    </button>
                  )}
                  <button
                    type="button"
                    className="btn btn-sm btn-primary"
                    disabled={busy !== null}
                    onClick={() => void doLaunch("run")}
                  >
                    {willQueue ? "Run (will queue)" : "Run"}
                  </button>
                </div>
              </section>
            )}

            <div className="tp-dates tk-muted">
              Created {formatDateTime(task.createdAt)} · Updated {formatDateTime(task.updatedAt)}
              {task.closedAt ? ` · Closed ${formatDateTime(task.closedAt)}` : ""}
              {task.priority !== "none" ? ` · ${PRIORITY_LABEL[task.priority]}` : ""}
            </div>
          </>
        )}
      </div>

      <footer className="tp-foot">
        <button type="button" className="btn btn-sm" onClick={() => setEditing(true)}>
          Edit
        </button>
        {src && (
          <button type="button" className="btn btn-sm" disabled={busy !== null} onClick={unlink}>
            Unlink
          </button>
        )}
        <button type="button" className="btn btn-sm btn-danger" disabled={busy !== null || !!activeRun} onClick={remove}>
          Delete
        </button>
        <span className="tp-spacer" />
        {activeRun ? (
          <button type="button" className="btn btn-sm btn-primary" onClick={() => onOpenRun(activeRun.id)}>
            Open run
          </button>
        ) : current && !canLaunch ? (
          <button type="button" className="btn btn-sm" onClick={() => onOpenRun(current.id)}>
            Open last run
          </button>
        ) : null}
        {closed ? (
          <button type="button" className="btn btn-sm" disabled={busy !== null} onClick={() => setStatus("todo")}>
            Reopen
          </button>
        ) : (
          <button type="button" className="btn btn-sm" disabled={busy !== null} onClick={() => setStatus("done")}>
            Mark done
          </button>
        )}
      </footer>

      {editing && (
        <NewTaskDialog
          projectId={task.projectId}
          taskId={task.id}
          onClose={() => setEditing(false)}
          onSaved={() => setEditing(false)}
        />
      )}
    </Shell>
  );
}

function Shell({
  refEl,
  headingId,
  onClose,
  children,
}: {
  refEl: RefObject<HTMLElement | null>;
  headingId: string;
  onClose: () => void;
  children: ReactNode;
}) {
  return (
    <>
      <div className="tp-scrim" onClick={onClose} aria-hidden />
      <aside
        ref={refEl}
        className="tp"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId}
        tabIndex={-1}
      >
        {children}
      </aside>
    </>
  );
}

interface MenuItem {
  key: string;
  label: string;
  dot?: string;
  checked: boolean;
  onPick: () => void;
}

function MenuList({ label, items, onEscape }: { label: string; items: MenuItem[]; onEscape: () => void }) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    ref.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
  }, []);
  return (
    <div
      ref={ref}
      className="menu tp-menu"
      role="menu"
      aria-label={label}
      onKeyDown={(e) => {
        const list = [...(ref.current?.querySelectorAll<HTMLElement>(".menu-item") ?? [])];
        const i = list.indexOf(document.activeElement as HTMLElement);
        if (e.key === "Escape") {
          e.preventDefault();
          e.stopPropagation();
          onEscape();
        } else if (e.key === "ArrowDown") {
          e.preventDefault();
          list[Math.min(list.length - 1, i + 1)]?.focus();
        } else if (e.key === "ArrowUp") {
          e.preventDefault();
          list[Math.max(0, i - 1)]?.focus();
        }
      }}
    >
      {items.map((it) => (
        <button key={it.key} type="button" role="menuitemradio" aria-checked={it.checked} className="menu-item" onClick={it.onPick}>
          <span className="menu-mark" aria-hidden>{it.checked ? "✓" : ""}</span>
          {it.dot && <span className="tp-dot" style={{ background: it.dot }} aria-hidden />}
          <span className="ellipsis">{it.label}</span>
        </button>
      ))}
      {items.length === 0 && <div className="menu-label">No other repos in this project</div>}
    </div>
  );
}

function PlanSection({ task }: { task: Task }) {
  const plan = useTaskPlan(task.id, task.updatedAt);
  const label =
    task.plan.kind === "file"
      ? `Plan · ${task.plan.path}`
      : task.source && !task.planOverridden
        ? `Description · synced from ${providerLabel(task.source.provider)}`
        : "Plan · written in Nodal";
  return (
    <section className="tp-section" aria-label="Plan">
      <span className="section-label">{label}</span>
      {plan.error ? (
        <div className="banner banner-warn">{plan.error}</div>
      ) : plan.data === undefined ? (
        <span className="tk-muted">Loading plan…</span>
      ) : plan.data.trim() ? (
        <SafeMarkdown text={plan.data} className="tp-plan" />
      ) : (
        <span className="tk-muted">No description.</span>
      )}
    </section>
  );
}

function AcceptanceSection({ task, onEdit }: { task: Task; onEdit: () => void }) {
  return (
    <section className="tp-section" aria-label="Acceptance criteria">
      <div className="tp-section-head">
        <span className="section-label">Acceptance criteria</span>
        <button type="button" className="btn btn-ghost btn-xs" onClick={onEdit}>
          Edit
        </button>
      </div>
      {task.acceptance.length ? (
        <ol className="tp-ac">
          {task.acceptance.map((a, i) => (
            <li key={i}>{a}</li>
          ))}
        </ol>
      ) : (
        <span className="tk-muted">None yet. The reviewer infers them from the plan and says so in its verdict.</span>
      )}
    </section>
  );
}

type RelChoice = "related" | "blocks" | "blocked_by";

function RelationsSection({ task, onOpenTask }: { task: Task; onOpenTask?: (id: string) => void }) {
  const push = useToast();
  const rels = useTaskRelations(task.id);
  const tasks = useTasks(task.projectId);
  const projects = useProjects();
  const project = projects.data?.find((p) => p.id === task.projectId);
  const [adding, setAdding] = useState(false);
  const [kind, setKind] = useState<RelChoice>("related");
  const [other, setOther] = useState("");

  const byId = new Map((tasks.data ?? []).map((t) => [t.id, t]));
  const name = (t: Task | undefined) => (t && project ? taskKey(project.key, t.number) : "?");
  const rows = (rels.data ?? []).map((r) => {
    const mine = r.taskId === task.id;
    const otherId = mine ? r.otherId : r.taskId;
    const label = r.kind === "related" ? "Related" : mine ? "Blocks" : "Blocked by";
    return { r, otherId, label, t: byId.get(otherId) };
  });

  const add = async () => {
    if (!other) return;
    try {
      if (kind === "blocked_by") await addTaskRelation(other, task.id, "blocks");
      else await addTaskRelation(task.id, other, kind as RelationKind);
      setAdding(false);
      setOther("");
      rels.refresh();
    } catch (e) {
      push("Couldn't add the relation", String(e), "danger");
    }
  };

  const remove = async (r: { taskId: string; otherId: string; kind: RelationKind }) => {
    try {
      await removeTaskRelation(r.taskId, r.otherId, r.kind);
      rels.refresh();
    } catch (e) {
      push("Couldn't remove the relation", String(e), "danger");
    }
  };

  const candidates = (tasks.data ?? []).filter((t) => t.id !== task.id).sort((a, b) => b.number - a.number);

  return (
    <section className="tp-section" aria-label="Relations">
      <div className="tp-section-head">
        <span className="section-label">Relations</span>
        {!adding && (
          <button type="button" className="btn btn-ghost btn-xs" onClick={() => setAdding(true)}>
            + Add
          </button>
        )}
      </div>
      {rows.length === 0 && !adding && <span className="tk-muted">No related tasks.</span>}
      {rows.map(({ r, otherId, label, t }) => (
        <div key={`${r.taskId}:${r.otherId}:${r.kind}`} className="tp-rel">
          <span className="tp-rel-kind">{label}</span>
          {t && <StatusRing status={t.status} size={10} />}
          <button
            type="button"
            className="tp-rel-title"
            disabled={!onOpenTask || !t}
            onClick={() => onOpenTask?.(otherId)}
          >
            <span className="mono tk-muted">{name(t)}</span>
            <span className="ellipsis">{t?.title ?? "Task not found"}</span>
          </button>
          {label === "Blocked by" && t && t.status !== "done" && <span className="tp-warn">not done</span>}
          <button type="button" className="icon-btn" aria-label={`Remove relation with ${name(t)}`} onClick={() => void remove(r)}>
            ✕
          </button>
        </div>
      ))}
      {adding && (
        <div className="tp-rel-add">
          <select className="input" value={kind} onChange={(e) => setKind(e.target.value as RelChoice)} aria-label="Relation kind">
            <option value="related">Related to</option>
            <option value="blocks">Blocks</option>
            <option value="blocked_by">Blocked by</option>
          </select>
          <select className="input tp-rel-task" value={other} onChange={(e) => setOther(e.target.value)} aria-label="Task">
            <option value="">Choose a task…</option>
            {candidates.map((t) => (
              <option key={t.id} value={t.id}>
                {name(t)} · {t.title}
              </option>
            ))}
          </select>
          <button type="button" className="btn btn-sm btn-primary" disabled={!other} onClick={() => void add()}>
            Add
          </button>
          <button type="button" className="btn btn-sm btn-ghost" onClick={() => setAdding(false)}>
            Cancel
          </button>
        </div>
      )}
    </section>
  );
}

function StepRow({
  run,
  n,
  onOpenRun,
  onOpenDiff,
}: {
  run: Run;
  n: number;
  onOpenRun: (id: string) => void;
  onOpenDiff: (id: string) => void;
}) {
  const start = run.launchedAt;
  const dur = start ? (run.finishedAt ?? Date.now()) - start : null;
  const v = run.verdict;
  const hasDiff = run.launchedAt != null && run.kind === "work";
  return (
    <li className={`tp-step ${run.kind === "review" ? "review" : ""}`}>
      <div className="tp-step-row">
        <span className="tp-step-n num">{n}</span>
        <ExecutorName executor={run.executor} />
        <span className={`tp-kind ${run.kind}`}>{run.kind === "review" ? "Review" : "Work"}</span>
        {run.parentRunId && run.kind === "work" && <span className="tk-muted tp-step-tag">hand-off</span>}
        <span className="tp-spacer" />
        <RunBadge run={run} />
        <span className="tk-muted num tp-step-dur">{dur != null ? formatDuration(dur) : "—"}</span>
      </div>
      {(run.summary || v || run.error || run.extraInstructions) && (
        <div className="tp-step-detail">
          {run.error && <div className="tp-step-err">{run.error}</div>}
          {v?.summary && <div>{v.summary}</div>}
          {!v && run.summary && <div>{run.summary}</div>}
          {v && v.unmet.length > 0 && (
            <div>
              <span className="tp-step-sub">Unmet</span>
              <ul>
                {v.unmet.map((u, i) => (
                  <li key={i}>{u}</li>
                ))}
              </ul>
            </div>
          )}
          {v && v.nits.length > 0 && (
            <details>
              <summary className="tp-step-sub">{v.nits.length} nit{v.nits.length === 1 ? "" : "s"}</summary>
              <ul>
                {v.nits.map((u, i) => (
                  <li key={i}>{u}</li>
                ))}
              </ul>
            </details>
          )}
          {run.extraInstructions && <div className="tk-muted">Extra: {run.extraInstructions}</div>}
        </div>
      )}
      <div className="tp-step-actions">
        {run.prUrl && (
          <button type="button" className="btn btn-ghost btn-xs" onClick={() => void openUrl(run.prUrl as string).catch(() => {})}>
            Open PR ↗
          </button>
        )}
        {run.branch && <span className="mono tk-muted ellipsis">{run.branch}</span>}
        <span className="tp-spacer" />
        {hasDiff && (
          <button type="button" className="btn btn-ghost btn-xs" onClick={() => onOpenDiff(run.id)}>
            View diff
          </button>
        )}
        <button type="button" className="btn btn-ghost btn-xs" onClick={() => onOpenRun(run.id)}>
          Open run
        </button>
      </div>
    </li>
  );
}
