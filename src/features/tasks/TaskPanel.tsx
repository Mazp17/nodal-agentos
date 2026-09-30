import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode, type RefObject } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  addTaskRelation,
  cleanupWorktree,
  deleteTask,
  removeTaskRelation,
  resolveMovedTask,
  unlinkTask,
  updateTask,
  worktreeStatus,
  type TaskPatch,
  type WorktreeStatus,
} from "../../domain/api";
import { isRunActive, useQueueSummary, useTaskRuns } from "../../domain/hooks/runs";
import {
  invalidate,
  KEYS,
  setData,
  useProjectList,
  useRepos,
  useTask,
  useTaskPlan,
  useTaskRelations,
  useTasks,
  useSettings,
  useWorktreeStatus,
} from "../../domain/hooks/store";
import { taskKey, TASK_STATUSES, type Executor, type RelationKind, type RunLight, type Task, type TaskStatus } from "../../domain/types";
import { formatDateTime, formatDuration, formatTokens } from "../../lib/format";
import { Kbd, KbdGroup } from "../../ui/Kbd";
import { SafeMarkdown } from "../../ui/Markdown";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useToast } from "../../ui/Toasts";
import { ExecutorName, ExecutorPicker, executorLabel, inheritedExecutor, sameExecutor } from "../executors";
import { SourceTab } from "../providers";
import { PhaseSegments, RunBadge, useRunActions, useRunView } from "../runs";
import { MergeDialog } from "./MergeDialog";
import { NewTaskDialog } from "./NewTaskDialog";
import { PriorityBars, StatusRing } from "./bits";
import { useAskLaunch } from "./LaunchPopover";
import { isClosed, PRIORITIES, PRIORITY_LABEL, providerLabel, STATUS_META } from "./status";
import { useLaunch } from "./useLaunch";
import "./tasks.css";

export interface TaskPanelProps {
  taskId: string;
  onClose: () => void;
  onOpenRun: (runId: string) => void;
  /** Opens a run's diff (RunDiffDrawer). */
  onOpenDiff: (runId: string) => void;
  /** Navigate to a related task; without it, relations aren't clickable. */
  onOpenTask?: (taskId: string) => void;
  /** "Review mapping" in the Source tab: Project settings → Sources. */
  onOpenSources?: (projectId: string) => void;
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

/** Task detail (drawer): status, repo, plan, criteria, relations, step chain and launch. */
export function TaskPanel({ taskId, onClose, onOpenRun, onOpenDiff, onOpenTask, onOpenSources }: TaskPanelProps) {
  const ref = useFocusTrap<HTMLElement>(onClose);
  const headingId = useId();
  const push = useToast();
  const launch = useLaunch();
  const askLaunch = useAskLaunch();
  const ask = useConfirm();
  const runActions = useRunActions();

  const taskQ = useTask(taskId);
  const task = taskQ.data;
  const projects = useProjectList();
  const repos = useRepos(task?.projectId ?? null);
  const runsQ = useTaskRuns(taskId);
  const summary = useQueueSummary();
  const globalExecutor = useSettings().data?.defaultExecutor ?? null;

  const [tab, setTab] = useState<"overview" | "source">("overview");
  const [menu, setMenu] = useState<"status" | "prio" | "repo" | "more" | null>(null);
  const [editing, setEditing] = useState(false);
  const [merging, setMerging] = useState(false);
  const [launchExec, setLaunchExec] = useState<Executor | null>(null);
  const [extra, setExtra] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  /** Why the backend rejected the non-forced "Clean up". */
  const [cleanupBlocked, setCleanupBlocked] = useState<string | null>(null);
  const wt = useWorktreeStatus(taskQ.data?.worktree ? taskId : null);

  useEffect(() => {
    setTab("overview");
    setLaunchExec(null);
    setExtra("");
    setMenu(null);
    setCleanupBlocked(null);
    setMerging(false);
  }, [taskId]);

  const closeMenu = useCallback(() => setMenu(null), []);
  const statusRef = useOutside(menu === "status", closeMenu);
  const repoRef = useOutside(menu === "repo", closeMenu);
  const prioRef = useOutside(menu === "prio", closeMenu);
  const moreRef = useOutside(menu === "more", closeMenu);
  const runBtnRef = useRef<HTMLButtonElement>(null);

  const project = projects.data?.find((p) => p.id === task?.projectId);
  const projectRepos = (repos.data ?? []).filter((r) => r.projectId === task?.projectId);
  const repo = projectRepos.find((r) => r.id === task?.repoId);
  const runs = runsQ.data ?? [];
  const chain = useMemo(() => [...(runsQ.data ?? [])].reverse(), [runsQ.data]);
  const current = runs[0];
  const activeRun = runs.find(isRunActive);
  const workRuns = runs.filter((r) => r.kind === "work" && r.launchedAt != null);
  const lastWork = workRuns[0];
  const heroRun = activeRun ?? current;

  if (!task) {
    return (
      <Shell refEl={ref} headingId={headingId} onClose={onClose}>
        <h2 id={headingId} className="sr-only">
          Task
        </h2>
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
  const inherited = inheritedExecutor(repo, project, globalExecutor);
  const assignee = task.assignee ?? inherited;
  const runExec = launchExec ?? assignee;
  const src = task.source;
  const closed = isClosed(task.status);
  const canLaunch = !!repo && !activeRun && !closed;
  const finish = lastWork?.finish ?? repo?.defaultFinish ?? "pr";
  const canMerge = !!task.worktree && !!repo && finish !== "pr" && !closed;
  const willQueue = summary.queued > 0 || (summary.capacity > 0 && summary.running >= summary.capacity);

  const act = async (label: string, fn: () => Promise<unknown>, okMsg?: [string, string?]) => {
    setBusy(label);
    try {
      await fn();
      if (okMsg) push(okMsg[0], okMsg[1], "ok");
      return true;
    } catch (e) {
      push(`Couldn't ${label}`, String(e), "danger");
      return false;
    } finally {
      setBusy(null);
      invalidate("tasks", "runs");
    }
  };

  const patch = (p: TaskPatch, label: string, ok?: [string, string?]) => act(label, () => updateTask(task.id, p), ok);

  const setAssignee = (a: Executor | null) => {
    if (a === null ? task.assignee === null : sameExecutor(a, task.assignee)) return;
    void patch({ assignee: a }, "change the executor", [
      `${key} now assigned to ${executorLabel(a ?? inherited)}`,
      a ? task.title : "Inherited default",
    ]);
  };
  const assigneeLocked = activeRun
    ? "Can't change the executor while a run is active"
    : busy !== null
      ? "Wait for the current action to finish"
      : undefined;

  const setStatus = (s: TaskStatus) => {
    setMenu(null);
    if (s !== task.status) void patch({ status: s }, "change the status", [`${key} → ${STATUS_META[s].label}`, task.title]);
  };

  const doLaunch = async (how: "run" | "handoff" | "review", anchor?: HTMLElement) => {
    const config = how === "run" ? await askLaunch({ name: key, title: task.title, repo, executor: runExec, anchor }) : null;
    if (how === "run" && !config) return;
    setBusy(how);
    const extraText = extra.trim() || null;
    const run = config
      ? await launch(task.id, key, {
          kind: "run",
          config,
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

  /** Without forcing, the backend refuses if there are unpushed commits or uncommitted changes. */
  const cleanUp = async () => {
    const w = task.worktree;
    if (!w) return;
    const ok = await ask({
      title: `Delete the worktree and branch ${w.branch}?`,
      body: (
        <>
          <span>Nodal refuses if the branch has unpushed commits or uncommitted changes.</span>
          <span className="confirm-path">{w.path}</span>
        </>
      ),
      confirmLabel: "Clean up",
    });
    if (!ok) return;
    setBusy("cleanup");
    try {
      await cleanupWorktree(task.id, false);
      setCleanupBlocked(null);
      push("Worktree cleaned up", w.branch, "ok");
    } catch (e) {
      setCleanupBlocked(String(e));
    } finally {
      setBusy(null);
      void invalidate("tasks", "runs");
    }
  };

  const cleanUpAnyway = async () => {
    const w = task.worktree;
    if (!w) return;
    // Busy right away: the await below leaves a window for a second click.
    setBusy("cleanup");
    // Fresh status: the polled one can be up to 20 s old.
    const s = await worktreeStatus(task.id).catch(() => wt.data);
    const losses = [
      s && s.unpushed > 0 ? `${s.unpushed} commit${s.unpushed === 1 ? "" : "s"} that exist nowhere else` : null,
      s?.dirty ? "uncommitted changes" : null,
    ].filter(Boolean);
    // Not busy during the confirm (modal: no double click possible) so focus
    // returns to the button on cancel; `act` sets it again.
    setBusy(null);
    const ok = await ask({
      title: `Force clean up ${w.branch}?`,
      body: (
        <>
          <span>
            This permanently deletes {losses.length ? losses.join(" and ") : "any unpublished work"} in the worktree and
            the branch. This can't be undone.
          </span>
          <span className="confirm-path">{w.path}</span>
        </>
      ),
      confirmLabel: "Delete permanently",
    });
    if (!ok) return;
    await act("clean up the worktree", async () => {
      await cleanupWorktree(task.id, true);
      setCleanupBlocked(null);
    }, ["Worktree cleaned up", w.branch]);
  };

  const remove = async () => {
    const ok = await ask({
      title: `Delete ${key}?`,
      body: `"${task.title}" and its plan are deleted. This can't be undone.`,
      confirmLabel: "Delete task",
    });
    if (!ok) return;
    await act("delete the task", async () => {
      await deleteTask(task.id);
      onClose();
    }, ["Task deleted", task.title]);
  };

  const unlink = async () => {
    if (!src) return;
    const ok = await ask({
      title: `Unlink ${src.identifier}?`,
      body: `It stays in ${providerLabel(src.provider)}. This task becomes local and stops syncing in both directions.`,
      confirmLabel: "Unlink",
    });
    if (!ok) return;
    await act("unlink the task", () => unlinkTask(task.id), [`${key} unlinked`, `${src.identifier} stays in ${providerLabel(src.provider)}.`]);
  };

  const provider = src ? providerLabel(src.provider) : null;
  const canDiffLast = !!lastWork && (!!task.worktree || lastWork.isolation !== "worktree");
  const mergeButton =
    canMerge && task.worktree ? (
      <button
        type="button"
        className="btn btn-sm btn-primary"
        disabled={busy !== null || !!activeRun}
        title={activeRun ? "Cancel the running step first" : `Land ${task.worktree.branch} on ${task.worktree.base}`}
        onClick={() => setMerging(true)}
      >
        {finish === "changes" ? "Commit & merge" : `Merge into ${task.worktree.base} & done`}
      </button>
    ) : null;
  /** Menu action: closes the menu first. */
  const picked = (fn: () => void) => () => {
    closeMenu();
    moreRef.current?.querySelector<HTMLElement>("button")?.focus();
    fn();
  };
  const refocus = (r: RefObject<HTMLDivElement | null>) => () => {
    closeMenu();
    r.current?.querySelector<HTMLElement>("button")?.focus();
  };

  return (
    <Shell refEl={ref} headingId={headingId} onClose={onClose}>
      <h2 id={headingId} className="sr-only">
        {key} · {task.title}
      </h2>
      <div className="tp-bar">
        {project && <span className="tp-proj-dot" style={{ background: project.color }} aria-hidden />}
        {project && <span className="tp-nowrap">{project.name}</span>}
        <span className="tp-sep" aria-hidden>
          /
        </span>
        <span className="mono tp-bar-key">{key}</span>
        {src ? (
          <>
            <span className="tp-sep" aria-hidden>
              ·
            </span>
            <button type="button" className="tp-link mono" onClick={() => void openUrl(src.url).catch(() => {})} title={src.url}>
              {src.identifier} ↗
            </button>
            <span className={`tp-sync ellipsis ${src.syncError ? "tp-danger" : ""}`}>
              {src.externalState ? `${src.externalState.name} · ` : ""}
              {src.syncError ? "Sync failed" : src.lastSyncedAt ? `Synced ${formatDateTime(src.lastSyncedAt)}` : "Not synced yet"}
            </span>
          </>
        ) : (
          <span className="tp-local">Local</span>
        )}
        <span className="tp-spacer" />
        <div className="tp-menu-wrap" ref={moreRef}>
          <button
            type="button"
            className="icon-btn tp-more"
            data-autofocus
            aria-haspopup="menu"
            aria-expanded={menu === "more"}
            aria-label="More actions"
            title="More actions"
            onClick={() => setMenu(menu === "more" ? null : "more")}
          >
            ⋯
          </button>
          {menu === "more" && (
            <MenuList
              label="More actions"
              className="tp-menu-end"
              onEscape={refocus(moreRef)}
              items={[
                { key: "edit", label: "Edit task…", onPick: picked(() => setEditing(true)) },
                closed
                  ? { key: "reopen", label: "Reopen", disabled: busy !== null, onPick: picked(() => setStatus("todo")) }
                  : {
                      key: "done",
                      label: canMerge ? "Mark done without merging" : "Mark done",
                      disabled: busy !== null,
                      onPick: picked(() => setStatus("done")),
                    },
                ...(canMerge && task.worktree
                  ? [
                      {
                        key: "merge",
                        label: finish === "changes" ? "Commit & merge…" : `Merge into ${task.worktree.base} & done…`,
                        disabled: busy !== null || !!activeRun,
                        onPick: picked(() => setMerging(true)),
                      },
                    ]
                  : []),
                ...(heroRun
                  ? [{ key: "run", label: activeRun ? "Open run" : "Open last run", onPick: picked(() => onOpenRun(heroRun.id)) }]
                  : []),
                ...(lastWork && canDiffLast
                  ? [{ key: "diff", label: "View diff", onPick: picked(() => onOpenDiff(lastWork.id)) }]
                  : []),
                ...(src
                  ? [{ key: "unlink", label: `Unlink from ${provider}…`, disabled: busy !== null, onPick: picked(() => void unlink()) }]
                  : []),
                {
                  key: "delete",
                  label: "Delete task…",
                  danger: true,
                  disabled: busy !== null || !!activeRun,
                  onPick: picked(() => void remove()),
                },
              ]}
            />
          )}
        </div>
        <button type="button" className="icon-btn" aria-label="Close task" title="Close (Esc)" onClick={onClose}>
          ✕
        </button>
      </div>

      <div className="tp-body">
        <div className="tp-top">
          <TitleField
            key={task.id}
            title={task.title}
            readOnly={!!src}
            readOnlyHint={provider ? `The title comes from ${provider}` : undefined}
            onSave={(title) => void patch({ title }, "rename the task")}
          />
          <div className="tp-meta">
            <div className="tp-menu-wrap" ref={statusRef}>
              <button
                type="button"
                className="btn btn-sm tp-chip-btn"
                aria-haspopup="menu"
                aria-expanded={menu === "status"}
                aria-label={`Status: ${STATUS_META[task.status].label}`}
                onClick={() => setMenu(menu === "status" ? null : "status")}
              >
                <StatusRing status={task.status} />
                {STATUS_META[task.status].label}
              </button>
              {menu === "status" && (
                <MenuList
                  label="Status"
                  onEscape={refocus(statusRef)}
                  items={TASK_STATUSES.map((s) => ({
                    key: s,
                    label: STATUS_META[s].label,
                    icon: <StatusRing status={s} />,
                    checked: s === task.status,
                    onPick: () => setStatus(s),
                  }))}
                />
              )}
            </div>
            <div className="tp-menu-wrap" ref={prioRef} title={provider ? `Priority comes from ${provider}` : undefined}>
              <button
                type="button"
                className="btn btn-sm tp-chip-btn"
                aria-haspopup="menu"
                aria-expanded={menu === "prio"}
                aria-label={`Priority: ${PRIORITY_LABEL[task.priority]}`}
                disabled={!!src}
                onClick={() => setMenu(menu === "prio" ? null : "prio")}
              >
                <PriorityBars priority={task.priority} />
                {task.priority === "none" ? "Priority" : PRIORITY_LABEL[task.priority]}
              </button>
              {menu === "prio" && (
                <MenuList
                  label="Priority"
                  onEscape={refocus(prioRef)}
                  items={PRIORITIES.map((p) => ({
                    key: p,
                    label: PRIORITY_LABEL[p],
                    icon: <PriorityBars priority={p} />,
                    checked: p === task.priority,
                    onPick: () => {
                      closeMenu();
                      if (p !== task.priority) void patch({ priority: p }, "change the priority");
                    },
                  }))}
                />
              )}
            </div>
            <div className="tp-menu-wrap" ref={repoRef}>
              <button
                type="button"
                className={`btn btn-sm tp-chip-btn mono ${repo ? "" : "tp-danger"}`}
                aria-haspopup="menu"
                aria-expanded={menu === "repo"}
                aria-label={`Repo: ${repo?.name ?? "removed"}. Move to another repo`}
                title={repo?.path}
                onClick={() => setMenu(menu === "repo" ? null : "repo")}
              >
                {repo ? repo.name : "Repo removed"}
                <span className="tp-caret" aria-hidden>
                  ▾
                </span>
              </button>
              {menu === "repo" && (
                <MenuList
                  label="Move to repo"
                  onEscape={refocus(repoRef)}
                  note={!projectRepos.some((r) => r.id !== task.repoId) ? "No other repos in this project" : undefined}
                  items={projectRepos.map((r) => ({
                    key: r.id,
                    label: r.name,
                    mono: true,
                    checked: r.id === task.repoId,
                    onPick: () => {
                      setMenu(null);
                      if (r.id !== task.repoId) void patch({ repoId: r.id }, "move the task", [`${key} moved`, `Now runs in ${r.name}`]);
                    },
                  }))}
                />
              )}
            </div>
            <ExecutorPicker
              variant="chip"
              repoId={repo?.id ?? null}
              projectId={task.projectId}
              value={task.assignee}
              inherited={inherited}
              label="Assignee"
              disabled={!!assigneeLocked}
              title={assigneeLocked ?? "Assignee"}
              onChange={setAssignee}
            />
            {task.labels.map((l) => (
              <span key={l} className="tp-label">
                {l}
              </span>
            ))}
          </div>
          {src?.syncError && (
            <div className="banner banner-error tp-banner" role="alert">
              <span className="tp-banner-mark" aria-hidden>
                !
              </span>
              <span className="tp-banner-text">
                {provider} sync failed: {src.syncError}
              </span>
            </div>
          )}
          {src?.moved && (
            <MovedBanner
              provider={provider ?? ""}
              from={src.moved.fromProject.name}
              to={src.moved.toProject?.name ?? "no project"}
              suggested={
                src.moved.suggestedRepoId === task.repoId
                  ? null
                  : (projectRepos.find((r) => r.id === src.moved?.suggestedRepoId)?.name ?? null)
              }
              current={repo?.name ?? null}
              busy={busy !== null}
              onMove={() =>
                void act("move the task", () => resolveMovedTask(task.id, "move"), [
                  `${key} moved`,
                  `Now runs in ${projectRepos.find((r) => r.id === src.moved?.suggestedRepoId)?.name ?? "the suggested repo"}`,
                ])
              }
              onKeep={() => void act("keep the task", () => resolveMovedTask(task.id, "keep"))}
            />
          )}
          {!repo && repos.data && (
            <div className="banner banner-error tp-banner">
              <span className="tp-banner-mark" aria-hidden>
                !
              </span>
              <span className="tp-banner-text">This task's repo was removed from the project. Choose another repo to run it.</span>
              <button type="button" className="btn btn-sm" onClick={() => setMenu("repo")}>
                Choose repo
              </button>
            </div>
          )}
          {src && (
            <div className="tp-tabs" role="tablist" aria-label="Task sections">
              {(
                [
                  ["overview", "Overview"],
                  ["source", provider ?? ""],
                ] as const
              ).map(([k, l]) => (
                <button
                  key={k}
                  type="button"
                  role="tab"
                  aria-selected={tab === k}
                  className={`tp-tab ${tab === k ? "on" : ""}`}
                  onClick={() => setTab(k)}
                >
                  {l}
                </button>
              ))}
            </div>
          )}
        </div>

        {tab === "source" && src ? (
          <SourceTab
            task={task}
            onTaskChange={(t) => {
              setData<Task[]>(KEYS.tasks, (ts) => ts.map((x) => (x.id === t.id ? t : x)));
              void invalidate("tasks");
            }}
            onOpenTask={onOpenTask}
            onReviewMapping={onOpenSources ? () => onOpenSources(task.projectId) : undefined}
          />
        ) : (
          <>
            {heroRun && (
              <RunHero
                run={heroRun}
                canDiff={heroRun.kind === "work" && heroRun.launchedAt != null && (!!task.worktree || heroRun.isolation !== "worktree")}
                prUrl={heroRun.prUrl ?? (heroRun.kind === "review" ? (lastWork?.prUrl ?? null) : null)}
                busy={busy !== null || runActions.busy}
                onOpenRun={onOpenRun}
                onOpenDiff={onOpenDiff}
                onStop={async (run) => {
                  const done = run.status === "queued" ? await runActions.remove({ run }, key) : await runActions.stop({ run }, key);
                  if (done) void invalidate("tasks");
                }}
                extra={mergeButton}
              />
            )}

            <PlanSection task={task} onEdit={() => setEditing(true)} />
            <AcceptanceSection
              key={task.id}
              task={task}
              onSave={(acceptance) => patch({ acceptance }, "update the criteria")}
            />

            <section className="tp-section" aria-label="Run history">
              <span className="section-label">Run history</span>
              {chain.length === 0 ? (
                <div className="tp-empty">No runs yet.</div>
              ) : (
                <ol className="tp-chain">
                  {chain.map((r, i) => (
                    <StepRow key={r.id} run={r} n={i + 1} onOpenRun={onOpenRun} onOpenDiff={onOpenDiff} />
                  ))}
                </ol>
              )}
            </section>

            <RelationsSection task={task} onOpenTask={onOpenTask} />

            {task.worktree && (
              <section className="tp-section" aria-label="Worktree">
                <span className="section-label">Worktree</span>
                <div className="tp-wt">
                  <div className="tp-wt-row">
                    <span className="tp-wt-k">Branch</span>
                    <span className="mono ellipsis">{wt.data?.branch ?? task.worktree.branch}</span>
                    <span className="tk-muted">from</span>
                    <span className="mono">{wt.data?.base ?? task.worktree.base}</span>
                  </div>
                  <div className="tp-wt-row">
                    <span className="tp-wt-k">State</span>
                    <WorktreeState status={wt.data} error={wt.error} />
                  </div>
                  <div className="tp-wt-row">
                    <span className="tp-wt-k">Path</span>
                    <span className="mono ellipsis tk-muted" title={task.worktree.path}>
                      {task.worktree.path}
                    </span>
                  </div>
                  <div className="tp-wt-actions">
                    {!heroRun && mergeButton}
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
                      onClick={() => void cleanUp()}
                    >
                      Clean up
                    </button>
                  </div>
                  {cleanupBlocked && (
                    <div className="banner banner-warn tp-wt-blocked" role="alert">
                      <span>{cleanupBlocked}</span>
                      <button
                        type="button"
                        className="btn btn-sm btn-danger"
                        disabled={!!activeRun || busy !== null}
                        onClick={() => void cleanUpAnyway()}
                      >
                        Clean up anyway
                      </button>
                    </div>
                  )}
                </div>
              </section>
            )}

            <div className="tp-dates">
              Created {formatDateTime(task.createdAt)} · Updated {formatDateTime(task.updatedAt)}
              {task.closedAt ? ` · Closed ${formatDateTime(task.closedAt)}` : ""}
            </div>
          </>
        )}
      </div>

      {canLaunch && (
        <footer className="tp-foot">
          <section className="tp-composer" aria-label="Launch">
            <textarea
              className="tp-extra"
              value={extra}
              onChange={(e) => setExtra(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter" || !(e.metaKey || e.ctrlKey) || e.nativeEvent.isComposing) return;
                e.preventDefault();
                if (busy === null) void doLaunch("run", runBtnRef.current ?? undefined);
              }}
              placeholder="Extra instructions for this run (optional)"
              aria-label="Extra instructions for this run"
            />
            <div className="tp-composer-bar">
              <ExecutorPicker
                variant="pill"
                caption="Executor"
                repoId={repo.id}
                projectId={task.projectId}
                value={launchExec}
                inherited={assignee}
                label="Executor for this run"
                onChange={setLaunchExec}
                dropUp
              />
              {lastWork && (
                <button type="button" className="tp-pill" disabled={busy !== null} onClick={() => void doLaunch("review")}>
                  Review now
                </button>
              )}
              {lastWork && (
                <button
                  type="button"
                  className="tp-pill"
                  disabled={busy !== null}
                  title="Next step on the same branch: gets the plan, criteria, previous summary and diff"
                  onClick={() => void doLaunch("handoff")}
                >
                  Hand off → {executorLabel(runExec)}
                </button>
              )}
              <span className="tp-spacer" />
              <button
                ref={runBtnRef}
                type="button"
                className="btn btn-primary tp-run"
                disabled={busy !== null}
                title={`Isolation, finish and review are chosen next · in ${repo.name}`}
                onClick={(e) => void doLaunch("run", e.currentTarget)}
              >
                {willQueue ? "Run (will queue)" : current ? "Run again" : "Run"}
                <KbdGroup aria-hidden>
                  <Kbd>⌘</Kbd>
                  <Kbd>↵</Kbd>
                </KbdGroup>
              </button>
            </div>
          </section>
          {willQueue && (
            <div className="tp-launch-warn">
              <span className="tp-launch-warn-mark" aria-hidden>
                !
              </span>
              <span>All run slots are busy: this run waits in the queue.</span>
            </div>
          )}
        </footer>
      )}

      {merging && task.worktree && (
        <MergeDialog
          taskId={task.id}
          worktree={task.worktree}
          message={`${key}: ${task.title}`}
          commitFirst={finish === "changes"}
          provider={provider}
          onClose={() => setMerging(false)}
          onMerged={(r) => {
            setMerging(false);
            const base = task.worktree?.base ?? "the base";
            if (r.outcome.kind === "merged") {
              const n = r.outcome.commits;
              push(`${key} merged into ${base}`, `${n} commit${n === 1 ? "" : "s"} → ${base} · Done`, "ok");
            }
            if (r.pushedTo) push(`${base} pushed`, r.pushedTo, "ok");
            if (r.pushError) push(`Couldn't push ${base}`, r.pushError, "danger");
            setCleanupBlocked(r.cleanupError);
            void invalidate("tasks", "runs");
          }}
          onHandOff={(instructions) => {
            setMerging(false);
            void (async () => {
              setBusy("handoff");
              await launch(task.id, key, { kind: "handoff", executor: { kind: "claude" }, extra: instructions });
              setBusy(null);
            })();
          }}
        />
      )}

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
  icon?: ReactNode;
  /** Set for radio items (status, priority, repo); plain actions leave it out. */
  checked?: boolean;
  mono?: boolean;
  danger?: boolean;
  disabled?: boolean;
  onPick: () => void;
}

function MenuList({
  label,
  items,
  onEscape,
  note,
  className = "",
}: {
  label: string;
  items: MenuItem[];
  onEscape: () => void;
  note?: string;
  className?: string;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    (el?.querySelector<HTMLElement>('[aria-checked="true"]') ?? el?.querySelector<HTMLElement>(".menu-item:not(:disabled)"))?.focus();
  }, []);
  return (
    <div
      ref={ref}
      className={`menu tp-menu ${className}`}
      role="menu"
      aria-label={label}
      onKeyDown={(e) => {
        const list = [...(ref.current?.querySelectorAll<HTMLElement>(".menu-item:not(:disabled)") ?? [])];
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
      {items.map((it) => {
        const radio = it.checked !== undefined;
        return (
          <button
            key={it.key}
            type="button"
            role={radio ? "menuitemradio" : "menuitem"}
            aria-checked={radio ? it.checked : undefined}
            className={`menu-item ${it.danger ? "tp-danger" : ""}`}
            disabled={it.disabled}
            onClick={it.onPick}
          >
            {radio && (
              <span className="menu-mark" aria-hidden>
                {it.checked ? "✓" : ""}
              </span>
            )}
            {it.icon}
            <span className={`ellipsis ${it.mono ? "mono" : ""}`}>{it.label}</span>
          </button>
        );
      })}
      {note && <div className="menu-label">{note}</div>}
    </div>
  );
}

/** Inline title: saves on Enter or blur; Escape reverts (and closes the panel once unchanged). */
function TitleField({
  title,
  readOnly,
  readOnlyHint,
  onSave,
}: {
  title: string;
  readOnly: boolean;
  readOnlyHint?: string;
  onSave: (title: string) => void;
}) {
  const [draft, setDraft] = useState(title);
  useEffect(() => setDraft(title), [title]);
  const commit = () => {
    const t = draft.trim();
    if (!t) setDraft(title);
    else if (t !== title) onSave(t);
  };
  return (
    <input
      className="tp-title"
      value={draft}
      readOnly={readOnly}
      title={readOnly ? readOnlyHint : draft}
      aria-label="Task title"
      onChange={(e) => setDraft(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter" && !e.nativeEvent.isComposing) {
          e.preventDefault();
          e.currentTarget.blur();
        } else if (e.key === "Escape" && draft !== title) {
          e.preventDefault();
          e.stopPropagation();
          setDraft(title);
        }
      }}
    />
  );
}

function PlanSection({ task, onEdit }: { task: Task; onEdit: () => void }) {
  const plan = useTaskPlan(task.id);
  const { refresh } = plan;
  // An imported task's plan is rematerialized by the sync: re-read it when the task changes
  // (not on mount: the hook already does that).
  const seen = useRef(task.updatedAt);
  useEffect(() => {
    if (seen.current === task.updatedAt) return;
    seen.current = task.updatedAt;
    refresh();
  }, [task.updatedAt, refresh]);
  const [label, from] =
    task.plan.kind === "file"
      ? ["Plan", task.plan.path]
      : task.source && !task.planOverridden
        ? ["Description", `synced from ${providerLabel(task.source.provider)}`]
        : ["Plan", "written in Nodal"];
  return (
    <section className="tp-section" aria-label="Plan">
      <div className="tp-section-head">
        <span className="section-label">{label}</span>
        <span className="tp-section-sub ellipsis">· {from}</span>
        <span className="tp-spacer" />
        <button type="button" className="btn btn-ghost btn-xs" onClick={onEdit}>
          Edit
        </button>
      </div>
      {plan.error ? (
        <div className="banner banner-warn">{plan.error}</div>
      ) : plan.data === undefined ? (
        <span className="tp-muted">Loading plan…</span>
      ) : plan.data.trim() ? (
        <SafeMarkdown text={plan.data} className="tp-plan" />
      ) : (
        <span className="tp-muted">No description.</span>
      )}
    </section>
  );
}

const sameList = (a: string[], b: string[]) => a.length === b.length && a.every((x, i) => x === b[i]);

/** Criteria edited in place: a row saves on blur or Enter, ✕ removes it, the last row adds one. */
function AcceptanceSection({ task, onSave }: { task: Task; onSave: (ac: string[]) => Promise<boolean> }) {
  const [items, setItems] = useState(task.acceptance);
  const [draft, setDraft] = useState("");
  const rootRef = useRef<HTMLElement>(null);
  const savedRef = useRef(task.acceptance);
  savedRef.current = task.acceptance;
  const pending = useRef(0);
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const saved = JSON.stringify(task.acceptance);
  // Take the saved list only while nobody is typing in it and no save is in flight: a reload
  // mid-edit would drop what's being typed and shift the rows under the pointer.
  const resync = useCallback(() => {
    if (!pending.current && !rootRef.current?.contains(document.activeElement)) setItems(savedRef.current);
  }, []);
  useEffect(resync, [saved, resync]);

  const commit = (next: string[]) => {
    const clean = next.map((s) => s.trim()).filter(Boolean);
    setItems(clean);
    if (sameList(clean, savedRef.current)) return;
    // One save at a time, in order: a blur and a ✕ in the same motion send two.
    pending.current++;
    queue.current = queue.current.then(() =>
      onSave(clean).then((ok) => {
        pending.current--;
        if (!ok && !pending.current) setItems(savedRef.current);
      }),
    );
  };
  const n = task.acceptance.length;

  return (
    <section
      ref={rootRef}
      className="tp-section tp-crits"
      aria-label="Acceptance criteria"
      onBlur={(e) => {
        if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setTimeout(resync);
      }}
    >
      <div className="tp-section-head">
        <span className="section-label">Acceptance criteria</span>
        {n > 0 && <span className="tp-section-sub num">{n}</span>}
      </div>
      {items.map((c, i) => (
        <div key={i} className="tp-crit">
          <span className="tp-box" aria-hidden />
          <input
            className="tp-crit-input"
            value={c}
            aria-label={`Criterion ${i + 1}`}
            onChange={(e) => setItems((l) => l.map((x, k) => (k === i ? e.target.value : x)))}
            onBlur={() => commit(items)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.nativeEvent.isComposing) {
                e.preventDefault();
                e.currentTarget.blur();
              } else if (e.key === "Escape" && c !== (task.acceptance[i] ?? "")) {
                e.preventDefault();
                e.stopPropagation();
                setItems((l) => l.map((x, k) => (k === i ? (task.acceptance[i] ?? "") : x)));
              }
            }}
          />
          <button
            type="button"
            className="icon-btn tp-crit-x"
            aria-label={`Remove criterion ${i + 1}`}
            title="Remove"
            onClick={() => commit(items.filter((_, k) => k !== i))}
          >
            ✕
          </button>
        </div>
      ))}
      {n === 0 && <div className="tp-empty">No criteria yet. The reviewer infers them from the plan and says so in its verdict.</div>}
      <div className="tp-crit">
        <span className="tp-box-plus" aria-hidden>
          +
        </span>
        <input
          className="tp-crit-input"
          value={draft}
          placeholder="Add a criterion, press Enter"
          aria-label="Add a criterion"
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => {
            if (!draft.trim()) return;
            commit([...items, draft]);
            setDraft("");
          }}
          onKeyDown={(e) => {
            if (e.key !== "Enter" || e.nativeEvent.isComposing || !draft.trim()) return;
            e.preventDefault();
            commit([...items, draft]);
            setDraft("");
          }}
        />
      </div>
    </section>
  );
}

type RelChoice = "related" | "blocks" | "blocked_by";

function RelationsSection({ task, onOpenTask }: { task: Task; onOpenTask?: (id: string) => void }) {
  const push = useToast();
  const rels = useTaskRelations(task.id);
  const tasks = useTasks(task.projectId);
  const projects = useProjectList();
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

/** Latest run up front: status, phase progress, what it's doing or what it produced, and its main actions. */
function RunHero({
  run: runProp,
  canDiff,
  prUrl,
  busy,
  onOpenRun,
  onOpenDiff,
  onStop,
  extra,
}: {
  run: RunLight;
  canDiff: boolean;
  /** The run's PR, or the reviewed work run's when this is a review. */
  prUrl: string | null;
  busy: boolean;
  onOpenRun: (id: string) => void;
  onOpenDiff: (id: string) => void;
  onStop: (run: RunLight) => Promise<void>;
  /** Extra action (merge) after the run's own. */
  extra?: ReactNode;
}) {
  const v = useRunView(runProp);
  // The shared store refreshes faster than the task's run list: read everything from it.
  const run = v.run;
  const active = v.tab === "active" || v.tab === "queued";
  const result = run.verdict?.summary ?? run.summary;
  const message = run.error
    ? null
    : v.phase === "running"
      ? run.kind === "review"
        ? "Reviewing the changes."
        : v.phaseName
          ? `${v.phaseName} in progress.`
          : "Working on it."
      : v.phase === "waiting"
        ? `${v.waitingFor === "permission prompt" ? "Waiting for a permission prompt" : "The agent needs input to continue"}. Open the run to answer it in Claude Code.`
        : v.phase === "queued"
          ? `Waiting in queue${v.queuePos != null ? ` at position #${v.queuePos}` : ""}. Starts when a slot frees up.`
          : v.phase === "awaiting"
            ? "Waiting for confirmation before it launches."
            : v.phase === "launching" || v.phase === "starting"
              ? v.label === "Not visible"
                ? "The session isn't visible in Claude Code yet."
                : "Starting the session…"
              : v.phase === "finished"
                ? result
                  ? null
                  : "Finished. Review the result before merging."
                : v.phase === "canceled"
                  ? v.label === "Stopped"
                    ? "Stopped before finishing."
                    : "Canceled before it launched."
                  : "The run failed.";

  return (
    <section className={`tp-hero tone-${v.tone}`} aria-label="Latest run">
      <div className="tp-hero-head">
        <RunBadge run={v} />
        <span className="mono tp-hero-id ellipsis">
          {executorLabel(run.executor)} · {run.kind === "review" ? "review" : "work"}
        </span>
        <span className="tp-spacer" />
        <span className="tp-hero-stats num">
          <span>{formatDuration(v.durationMs)}</span>
          {v.tokens != null && <span>{formatTokens(v.tokens)} tok</span>}
        </span>
      </div>
      <PhaseSegments run={v} />
      {run.error ? (
        <div className="tp-hero-msg tp-step-err">{run.error}</div>
      ) : message ? (
        <div className="tp-hero-msg">{message}</div>
      ) : (
        result && <SafeMarkdown text={result} className="md-compact tp-hero-msg" breaks />
      )}
      {run.verdict && run.verdict.unmet.length > 0 && (
        <span className="tp-warn">
          {run.verdict.unmet.length} unmet criteri{run.verdict.unmet.length === 1 ? "on" : "a"}
        </span>
      )}
      <div className="tp-hero-actions">
        {!active && prUrl ? (
          <>
            <button type="button" className="btn btn-sm btn-primary" onClick={() => void openUrl(prUrl).catch(() => {})}>
              Open PR ↗
            </button>
            <button type="button" className="btn btn-sm" onClick={() => onOpenRun(run.id)}>
              View run
            </button>
          </>
        ) : (
          <button type="button" className="btn btn-sm btn-primary" onClick={() => onOpenRun(run.id)}>
            Open run
          </button>
        )}
        {canDiff && (
          <button type="button" className="btn btn-sm" onClick={() => onOpenDiff(run.id)}>
            View diff
          </button>
        )}
        {active && (
          <button type="button" className="btn btn-sm btn-danger" disabled={busy} onClick={() => void onStop(run)}>
            {run.status === "queued" ? "Cancel" : "Stop"}
          </button>
        )}
        {extra}
      </div>
    </section>
  );
}

function StepRow({
  run,
  n,
  onOpenRun,
  onOpenDiff,
}: {
  run: RunLight;
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
      {(run.summary || v || run.error) && (
        <div className="tp-step-detail">
          {run.error && <div className="tp-step-err">{run.error}</div>}
          {v?.summary && <SafeMarkdown text={v.summary} className="md-compact" breaks />}
          {!v && run.summary && <SafeMarkdown text={run.summary} className="md-compact" breaks />}
          {v && v.unmet.length > 0 && (
            <div>
              <span className="tp-step-sub">Unmet</span>
              <ul>
                {v.unmet.map((u, i) => (
                  <li key={i}>
                    <SafeMarkdown text={u} className="md-compact" breaks />
                  </li>
                ))}
              </ul>
            </div>
          )}
          {v && v.nits.length > 0 && (
            <details>
              <summary className="tp-step-sub">{v.nits.length} nit{v.nits.length === 1 ? "" : "s"}</summary>
              <ul>
                {v.nits.map((u, i) => (
                  <li key={i}>
                    <SafeMarkdown text={u} className="md-compact" breaks />
                  </li>
                ))}
              </ul>
            </details>
          )}
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

/** "2 ahead · 1 unpushed · Uncommitted changes" for the task's worktree. */
function WorktreeState({ status, error }: { status: WorktreeStatus | undefined; error: string | null }) {
  if (!status) return <span className="tk-muted">{error ? `Couldn't read git status: ${error}` : "Checking…"}</span>;
  if (!status.exists) return <span className="tp-danger">Worktree folder is missing</span>;
  const parts: ReactNode[] = [];
  parts.push(
    <span key="ahead" className="num">
      {status.ahead} commit{status.ahead === 1 ? "" : "s"} ahead
    </span>,
  );
  if (status.unpushed > 0) {
    parts.push(
      <span key="unpushed" className="tp-warn num">
        {status.unpushed} unpushed
      </span>,
    );
  }
  if (status.dirty) {
    parts.push(
      <span key="dirty" className="tp-warn">
        Uncommitted changes
      </span>,
    );
  }
  if (status.unpushed === 0 && !status.dirty) parts.push(<span key="clean" className="tk-muted">Clean</span>);
  return (
    <span className="tp-wt-state">
      {parts.flatMap((p, i) => (i ? [<span key={`sep-${i}`} className="tk-muted" aria-hidden>·</span>, p] : [p]))}
    </span>
  );
}

/** The issue changed project in the provider: move the task to the repo it maps to, or keep it. */
function MovedBanner({
  provider,
  from,
  to,
  suggested,
  current,
  busy,
  onMove,
  onKeep,
}: {
  provider: string;
  from: string;
  to: string;
  /** Repo the rules would map it to; `null` if none applies (or it no longer exists). */
  suggested: string | null;
  current: string | null;
  busy: boolean;
  onMove: () => void;
  onKeep: () => void;
}) {
  return (
    <div className="banner banner-warn tp-banner tp-moved" role="status">
      <span className="tp-moved-text">
        Moved from {from} to {to} in {provider}
      </span>
      {suggested && (
        <button type="button" className="btn btn-sm" disabled={busy} onClick={onMove}>
          Move to {suggested}
        </button>
      )}
      <button type="button" className="btn btn-sm btn-ghost" disabled={busy} onClick={onKeep}>
        {current ? `Keep in ${current}` : "Keep here"}
      </button>
    </div>
  );
}
