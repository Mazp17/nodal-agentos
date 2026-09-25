import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useRef,
  useState,
  type ClipboardEvent,
  type FormEvent,
  type KeyboardEvent,
} from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { createTask, getTask, readTaskPlan, updateTask, type NewTask, type PlanInput, type TaskPatch } from "../../domain/api";
import { invalidate, useProjectList, useRepos, useSettings } from "../../domain/hooks/store";
import { taskKey, type Executor, type Finish, type Isolation, type Priority, type Task } from "../../domain/types";
import { SafeMarkdown } from "../../ui/Markdown";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useToast } from "../../ui/Toasts";
import { ExecutorPicker, executorLabel, inheritedExecutor, sameExecutor } from "../executors";
import { PriorityBars } from "./bits";
import {
  FINISH_HINT,
  FINISH_LABEL,
  ISOLATION_HINT,
  ISOLATION_LABEL,
  PRIORITIES,
  PRIORITY_LABEL,
  providerLabel,
} from "./status";
import { useLaunch } from "./useLaunch";
import "./tasks.css";

export interface NewTaskDialogProps {
  /** The view's project; `null` (All projects) lets the user pick one. When editing, the task's wins. */
  projectId: string | null;
  /** When set, edits that task. */
  taskId?: string;
  /** Preselected repo (e.g. the board's active filter). */
  defaultRepoId?: string | null;
  onClose: () => void;
  onSaved: (task: Task) => void;
  /** When set, offers "Add repo…" (handled by the project's Settings). */
  onAddRepo?: (projectId: string) => void;
}

type PlanKind = PlanInput["kind"];
type Menu = "repo" | "prio" | null;

const FINISHES: Finish[] = ["changes", "commit", "pr"];
const ISOLATIONS: Isolation[] = ["worktree", "in_place"];
const FINISH_OUTCOME: Record<Finish, string> = { changes: "leaves the changes", commit: "commits", pr: "opens a PR" };
const PLAN_TEMPLATE = "## Goal\n\n\n## Steps\n\n1. \n\n## Acceptance criteria\n\n- ";

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const cleanList = (l: string[]) => l.map((s) => s.trim()).filter(Boolean);
/** Strips a list marker (`-`, `*`, `1.`) and a checkbox (`[ ]`, `[x]`). */
const stripBullet = (l: string) => l.replace(/^\s*(?:[-*]|\d+\.)\s*(?:\[[ xX]\]\s*)?/, "").trim();

/** Bullets under a "Done when" / "Acceptance criteria" heading of the plan. */
function planCriteria(text: string): string[] {
  let inside = false;
  const out: string[] = [];
  for (const line of text.split("\n")) {
    if (/^#{1,4}\s/.test(line)) {
      inside = /acceptance|done when|criteria/i.test(line);
      continue;
    }
    const m = inside && line.match(/^\s*(?:[-*]|\d+\.)\s+(?:\[[ xX]\]\s*)?(.+)/);
    if (m) out.push(m[1].trim());
  }
  return out;
}

/** Too short or too vague for a reviewer to check. */
function isVague(c: string) {
  const x = c.trim();
  return !!x && (x.split(/\s+/).length < 3 || /\b(works?|properly|correctly|better|good|nice|clean|fine)\b/i.test(x));
}

function useOutside(active: boolean, close: () => void) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!active) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [active, close]);
  return ref;
}

/** Create or edit a task: repo (required), title, plan, done-when criteria and execution options. */
export function NewTaskDialog({ projectId, taskId, defaultRepoId, onClose, onSaved, onAddRepo }: NewTaskDialogProps) {
  const ref = useFocusTrap<HTMLFormElement>(onClose);
  const titleId = useId();
  const push = useToast();
  const launch = useLaunch();
  const projects = useProjectList();
  const allRepos = useRepos(null);
  const settings = useSettings();
  const titleRef = useRef<HTMLInputElement>(null);

  const [original, setOriginal] = useState<Task | null>(null);
  const [loaded, setLoaded] = useState(!taskId);
  const [pid, setPid] = useState<string | null>(projectId);
  const [repoId, setRepoId] = useState<string | null>(defaultRepoId ?? null);
  const [title, setTitle] = useState("");
  const [kind, setKind] = useState<PlanKind>("text");
  const [tab, setTab] = useState<"write" | "preview">("write");
  const [text, setText] = useState("");
  const [origText, setOrigText] = useState("");
  const [path, setPath] = useState("");
  const [priority, setPriority] = useState<Priority>("none");
  const [labels, setLabels] = useState<string[]>([]);
  const [labelDraft, setLabelDraft] = useState("");
  const [acceptance, setAcceptance] = useState<string[]>([]);
  const [critDraft, setCritDraft] = useState("");
  const [assignee, setAssignee] = useState<Executor | null>(null);
  const [isolation, setIsolation] = useState<Isolation | null>(null);
  const [finish, setFinish] = useState<Finish | null>(null);
  const [review, setReview] = useState<boolean | null>(null);
  const [execOpen, setExecOpen] = useState(false);
  const [menu, setMenu] = useState<Menu>(null);
  const [tried, setTried] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const closeMenu = useCallback(() => setMenu(null), []);
  const repoMenuRef = useOutside(menu === "repo", closeMenu);
  const prioMenuRef = useOutside(menu === "prio", closeMenu);

  // Editing: preload the task and, for a text plan, its content.
  useEffect(() => {
    if (!taskId) return;
    let alive = true;
    (async () => {
      try {
        const t = await getTask(taskId);
        const planText = t.plan.kind === "text" ? await readTaskPlan(t.id) : "";
        if (!alive) return;
        setOriginal(t);
        setPid(t.projectId);
        setRepoId(t.repoId);
        setTitle(t.title);
        setKind(t.plan.kind);
        setText(planText);
        setOrigText(planText);
        setPath(t.plan.kind === "file" ? t.plan.path : "");
        setPriority(t.priority);
        setLabels(t.labels);
        setAcceptance(t.acceptance);
        setAssignee(t.assignee);
        setIsolation(t.isolation);
        setFinish(t.finish);
        setReview(t.review);
        setLoaded(true);
      } catch (e) {
        if (alive) setError(String(e));
      }
    })();
    return () => {
      alive = false;
    };
  }, [taskId]);

  // No project (All projects): the requested repo's, or else the first one.
  useEffect(() => {
    if (pid || !projects.data?.length || allRepos.data === undefined) return;
    const fromRepo = allRepos.data.find((r) => r.id === defaultRepoId)?.projectId;
    setPid(fromRepo ?? projects.data[0].id);
  }, [pid, projects.data, allRepos.data, defaultRepoId]);

  // Editing: focus the title once loaded (when it can be edited).
  useEffect(() => {
    if (loaded && taskId && !original?.source) titleRef.current?.focus();
  }, [loaded, taskId, original]);

  const project = projects.data?.find((p) => p.id === pid) ?? null;
  const sortedRepos = useMemo(
    () => [...(allRepos.data ?? [])].sort((a, b) => a.position - b.position),
    [allRepos.data],
  );
  const repos = useMemo(() => sortedRepos.filter((r) => r.projectId === pid), [sortedRepos, pid]);
  const repo = repos.find((r) => r.id === repoId) ?? null;
  // Creating from All projects: the repo menu lists every project's repos.
  const pickProject = projectId === null && !original && (projects.data?.length ?? 0) > 1;
  const menuGroups = useMemo(() => {
    const list = pickProject ? (projects.data ?? []) : project ? [project] : [];
    return list
      .map((p) => ({ project: p, repos: sortedRepos.filter((r) => r.projectId === p.id) }))
      .filter((g) => g.repos.length > 0 || pickProject || g.project.id === pid);
  }, [pickProject, projects.data, project, sortedRepos, pid]);

  // Default repo: the requested one, or the project's first.
  useEffect(() => {
    if (!loaded || taskId || !pid) return;
    if (repoId && repos.some((r) => r.id === repoId)) return;
    if (repos.length) setRepoId(repos[0].id);
    else if (allRepos.data) setRepoId(null);
  }, [loaded, taskId, pid, repoId, repos, allRepos.data]);

  const imported = !!original?.source;
  const inherited = inheritedExecutor(repo, project, settings.data?.defaultExecutor);
  const effExecutor = assignee ?? inherited;
  const isWorkflow = effExecutor.kind === "workflow";
  const effIsolation = isolation ?? repo?.defaultIsolation ?? "worktree";
  const effFinish = finish ?? repo?.defaultFinish ?? "pr";
  const effReview = review ?? repo?.defaultReview ?? true;
  const execCustom = assignee !== null || isolation !== null || finish !== null || review !== null;

  const titleErr = tried && !title.trim();
  const planErr = tried && (kind === "text" ? !text.trim() : !path.trim());
  const repoErr = tried && !repo;
  const missing = [!repo && "repo", !title.trim() && "title", (kind === "text" ? !text.trim() : !path.trim()) && "plan"].filter(
    Boolean,
  ) as string[];

  const suggestions = useMemo(() => {
    if (kind !== "text") return [];
    const have = new Set(acceptance.map((c) => c.trim().toLowerCase()));
    return [...new Set(planCriteria(text))].filter((c) => !have.has(c.toLowerCase()));
  }, [kind, text, acceptance]);
  const critCount = acceptance.filter((c) => c.trim()).length;

  const pickRepo = (id: string, projectOf: string) => {
    setMenu(null);
    if (id === repoId) return;
    if (projectOf !== pid) setPid(projectOf);
    setRepoId(id);
    // The `.md` is relative to the previous repo.
    setPath("");
    setError(null);
  };

  const browse = async () => {
    if (!repo) return;
    setError(null);
    try {
      const picked = await open({
        defaultPath: repo.path,
        multiple: false,
        directory: false,
        title: `Choose a plan in ${repo.name}`,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (typeof picked !== "string") return;
      const root = repo.path.replace(/\/+$/, "");
      if (!picked.startsWith(`${root}/`)) {
        setError(`The plan file must be inside ${repo.name} (${root}).`);
        return;
      }
      if (!picked.toLowerCase().endsWith(".md")) {
        setError("The plan must be a Markdown (.md) file.");
        return;
      }
      setPath(picked.slice(root.length + 1));
    } catch (e) {
      setError(String(e));
    }
  };

  const addLabel = () => {
    const parts = cleanList(labelDraft.toLowerCase().split(","));
    if (parts.length) setLabels((l) => [...l, ...parts.filter((p) => !l.includes(p))]);
    setLabelDraft("");
  };

  const onLabelKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter" || e.key === ",") {
      e.preventDefault();
      addLabel();
    } else if (e.key === "Backspace" && !labelDraft && labels.length) {
      setLabels((l) => l.slice(0, -1));
    }
  };

  const addCriteria = (items: string[]) => {
    const add = cleanList(items);
    if (add.length) setAcceptance((a) => [...a, ...add]);
  };
  const setAc = (i: number, v: string) => setAcceptance((a) => a.map((x, k) => (k === i ? v : x)));
  const acRefs = useRef<(HTMLInputElement | null)[]>([]);
  const draftRef = useRef<HTMLInputElement>(null);
  const removeAc = (i: number) => {
    setAcceptance((a) => a.filter((_, k) => k !== i));
    // Keep focus in the list: the previous row, or the draft when the first one goes.
    const prev = i > 0 ? acRefs.current[i - 1] : draftRef.current;
    prev?.focus();
  };

  const onDraftKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key !== "Enter" || e.metaKey) return;
    e.preventDefault();
    addCriteria([stripBullet(critDraft)]);
    setCritDraft("");
  };
  // A pasted list becomes one criterion per line.
  const onDraftPaste = (e: ClipboardEvent<HTMLInputElement>) => {
    const pasted = e.clipboardData.getData("text");
    if (!pasted.includes("\n")) return;
    e.preventDefault();
    addCriteria([stripBullet(critDraft), ...pasted.split("\n").map(stripBullet)]);
    setCritDraft("");
  };

  /** `null` = inherit: picking the repo's default goes back to inheriting. */
  const choose = <T,>(value: T, def: T | undefined, set: (v: T | null) => void) => set(value === def ? null : value);

  const planInput = (): PlanInput => (kind === "text" ? { kind: "text", text } : { kind: "file", path: path.trim() });

  const submit = async (run: boolean) => {
    setTried(true);
    if (!pid || missing.length) return;
    if (saving || !repo) return;
    setSaving(true);
    setError(null);
    const ac = cleanList([...acceptance, stripBullet(critDraft)]);
    const pendingLabel = cleanList(labelDraft.toLowerCase().split(",")).filter((l) => !labels.includes(l));
    const allLabels = [...labels, ...pendingLabel];
    try {
      let saved: Task;
      if (original) {
        const patch: TaskPatch = {};
        if (repo.id !== original.repoId) patch.repoId = repo.id;
        const planChanged =
          kind !== original.plan.kind ||
          (kind === "text" ? text !== origText : original.plan.kind === "file" && path.trim() !== original.plan.path);
        if (planChanged) patch.plan = planInput();
        if (!imported) {
          if (title.trim() !== original.title) patch.title = title.trim();
          if (priority !== original.priority) patch.priority = priority;
          if (!same(allLabels, original.labels)) patch.labels = allLabels;
        }
        if (!same(ac, original.acceptance)) patch.acceptance = ac;
        if (!same(assignee, original.assignee)) patch.assignee = assignee;
        if (isolation !== original.isolation) patch.isolation = isolation;
        if (finish !== original.finish) patch.finish = finish;
        if (review !== original.review) patch.review = review;
        saved = Object.keys(patch).length ? await updateTask(original.id, patch) : original;
        push("Task updated", saved.title, "ok");
      } else {
        const input: NewTask = {
          projectId: pid,
          repoId: repo.id,
          title: title.trim(),
          plan: planInput(),
          priority,
          labels: allLabels,
          acceptance: ac,
          assignee,
          isolation,
          finish,
          review,
        };
        saved = await createTask(input);
        const key = project ? taskKey(project.key, saved.number) : saved.title;
        if (run) await launch(saved.id, key, { kind: "run" });
        else push("Task created", `${key} · ${saved.title}`, "ok");
      }
      // Wait for the refetch: the panel that opens looks the task up in the shared list.
      await invalidate("tasks");
      onSaved(saved);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    void submit(false);
  };

  // ⌘↵ saves from anywhere; a plain Enter in a text field never submits the form.
  const onFormKey = (e: KeyboardEvent<HTMLFormElement>) => {
    // Escape closes an open menu first, not the dialog (the focus trap listens on document).
    if (e.key === "Escape" && menu) {
      e.preventDefault();
      e.stopPropagation();
      setMenu(null);
      return;
    }
    if (e.key !== "Enter") return;
    if (e.metaKey) {
      e.preventDefault();
      if (loaded) void submit(false);
    } else if (e.target instanceof HTMLInputElement) {
      e.preventDefault();
    }
  };

  const noRepos = allRepos.data !== undefined && pid !== null && repos.length === 0;
  const lines = text.split("\n").length;
  const planHeight = Math.min(340, Math.max(150, lines * 19 + 24));
  const repoDefault = (custom: boolean) => (repo ? (custom ? "differs from repo default" : "repo default") : "");

  const footNote =
    tried && missing.length
      ? `Missing: ${missing.join(", ")}`
      : original
        ? "Changes apply to the next run"
        : repo
          ? `${executorLabel(effExecutor)} runs ${isWorkflow || effIsolation === "worktree" ? "in a worktree" : "in your checkout"} → ${FINISH_OUTCOME[effFinish]}`
          : "";

  const execSummary = [
    executorLabel(effExecutor),
    isWorkflow ? null : ISOLATION_LABEL[effIsolation],
    FINISH_LABEL[effFinish],
    effReview ? "Review" : null,
  ]
    .filter(Boolean)
    .join(" · ");

  return (
    <>
      <div className="dlg-scrim" onClick={onClose} aria-hidden />
      <form
        ref={ref}
        className="dlg"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onSubmit={onSubmit}
        onKeyDown={onFormKey}
      >
        <header className="dlg-head">
          <h2 id={titleId} className="dlg-title">
            {original ? "Edit task" : "New task"}
          </h2>
          <span className="nt-in">in</span>
          <div className="nt-anchor" ref={repoMenuRef}>
            <button
              type="button"
              className={`nt-repo ${repoErr ? "err" : ""}`}
              aria-haspopup="menu"
              aria-expanded={menu === "repo"}
              aria-label={`Repo: ${repo ? `${project?.name ?? ""} / ${repo.name}` : "none"}`}
              disabled={!loaded}
              onClick={() => setMenu(menu === "repo" ? null : "repo")}
            >
              {project && <span className="dlg-proj-dot" style={{ background: project.color }} aria-hidden />}
              {project && <span className="nt-repo-proj">{project.name} /</span>}
              <span className={`nt-repo-name ${repo ? "" : "empty"}`}>{repo?.name ?? "Choose repo"}</span>
              <span className="nt-caret" aria-hidden>
                ▼
              </span>
            </button>
            {menu === "repo" && (
              <div className="menu nt-menu nt-repo-menu" role="menu" aria-label="Repo">
                {menuGroups.map((g) => (
                  <div key={g.project.id} role="group" aria-label={g.project.name}>
                    {pickProject && (
                      <div className="menu-label nt-menu-proj">
                        <span className="dlg-proj-dot" style={{ background: g.project.color }} aria-hidden />
                        {g.project.name}
                      </div>
                    )}
                    {g.repos.map((r) => (
                      <button
                        key={r.id}
                        type="button"
                        role="menuitemradio"
                        aria-checked={r.id === repoId}
                        className="menu-item nt-repo-item"
                        onClick={() => pickRepo(r.id, g.project.id)}
                      >
                        <span className="menu-mark" aria-hidden>
                          {r.id === repoId ? "✓" : ""}
                        </span>
                        <span className="mono">{r.name}</span>
                        <span className="nt-repo-path mono ellipsis">{r.path}</span>
                      </button>
                    ))}
                  </div>
                ))}
                {onAddRepo && pid && (
                  <>
                    <div className="nt-sep" />
                    <button
                      type="button"
                      role="menuitem"
                      className="menu-item nt-add-repo"
                      onClick={() => {
                        setMenu(null);
                        onAddRepo(pid);
                      }}
                    >
                      Add repo…
                    </button>
                  </>
                )}
              </div>
            )}
          </div>
          <span className="dlg-spacer" />
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>

        {!loaded ? (
          <div className="dlg-body">
            {error ? <div className="banner banner-error">{error}</div> : <span className="tk-muted">Loading task…</span>}
          </div>
        ) : (
          <div className="dlg-body nt-body">
            {(repoErr || noRepos) && (
              <div className="nt-err nt-repo-err">
                {noRepos ? "This project has no repos yet. Add one from the repo menu." : "Choose which repo this task runs in."}
              </div>
            )}

            <section className="nt-sec nt-top">
              <input
                ref={titleRef}
                className="nt-title"
                data-autofocus={taskId ? undefined : true}
                value={title}
                disabled={imported}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="Task title"
                aria-label="Title"
                aria-invalid={titleErr || undefined}
              />
              {titleErr && <span className="nt-err nt-title-err">Give the task a title.</span>}
              {imported && (
                <span className="tk-hint">Title, priority and labels come from {providerLabel(original?.source?.provider ?? "")}.</span>
              )}
              <div className="nt-meta">
                <div className="nt-anchor" ref={prioMenuRef}>
                  <button
                    type="button"
                    className="nt-chip"
                    aria-haspopup="menu"
                    aria-expanded={menu === "prio"}
                    aria-label={`Priority: ${PRIORITY_LABEL[priority]}`}
                    disabled={imported}
                    onClick={() => setMenu(menu === "prio" ? null : "prio")}
                  >
                    <PriorityBars priority={priority} />
                    {priority === "none" ? "Priority" : PRIORITY_LABEL[priority]}
                    <span className="nt-caret" aria-hidden>
                      ▼
                    </span>
                  </button>
                  {menu === "prio" && (
                    <div className="menu nt-menu nt-prio-menu" role="menu" aria-label="Priority">
                      {PRIORITIES.map((p) => (
                        <button
                          key={p}
                          type="button"
                          role="menuitemradio"
                          aria-checked={priority === p}
                          className="menu-item"
                          onClick={() => {
                            setPriority(p);
                            setMenu(null);
                          }}
                        >
                          <PriorityBars priority={p} />
                          <span className="nt-grow">{PRIORITY_LABEL[p]}</span>
                          <span className="menu-mark" aria-hidden>
                            {priority === p ? "✓" : ""}
                          </span>
                        </button>
                      ))}
                    </div>
                  )}
                </div>
                {labels.map((l) => (
                  <span key={l} className="nt-label">
                    {l}
                    {!imported && (
                      <button
                        type="button"
                        aria-label={`Remove label ${l}`}
                        onClick={() => setLabels((x) => x.filter((y) => y !== l))}
                      >
                        ✕
                      </button>
                    )}
                  </span>
                ))}
                {!imported && (
                  <input
                    className="nt-label-input"
                    value={labelDraft}
                    onChange={(e) => setLabelDraft(e.target.value)}
                    onKeyDown={onLabelKey}
                    onBlur={addLabel}
                    placeholder="+ Label"
                    aria-label="Add label"
                  />
                )}
              </div>
            </section>

            <section className="nt-sec">
              <div className="nt-sec-head">
                <span className="nt-sec-title">Plan</span>
                {kind === "text" && (
                  <div className="nt-tabs" role="tablist" aria-label="Plan view">
                    {(
                      [
                        ["write", "Write"],
                        ["preview", "Preview"],
                      ] as const
                    ).map(([k, l]) => (
                      <button
                        key={k}
                        type="button"
                        role="tab"
                        aria-selected={tab === k}
                        className={`nt-tab ${tab === k ? "on" : ""}`}
                        onClick={() => setTab(k)}
                      >
                        {l}
                      </button>
                    ))}
                  </div>
                )}
                <span className="nt-grow" />
                {kind === "text" && !text.trim() && (
                  <button
                    type="button"
                    className="nt-soft-btn"
                    onClick={() => {
                      setText(PLAN_TEMPLATE);
                      setTab("write");
                    }}
                  >
                    Insert template
                  </button>
                )}
                <button type="button" className="nt-link" onClick={() => setKind(kind === "text" ? "file" : "text")}>
                  {kind === "text" ? "Use a .md from the repo" : "Write it here instead"}
                </button>
              </div>
              {kind === "text" && tab === "write" && (
                <textarea
                  className={`nt-plan ${planErr ? "err" : ""}`}
                  style={{ height: planHeight }}
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                  placeholder="What should change, and how? Steps, files, constraints."
                  aria-label="Plan (markdown)"
                  aria-invalid={planErr || undefined}
                />
              )}
              {kind === "text" && tab === "preview" && (
                <div className="nt-preview">
                  {text.trim() ? <SafeMarkdown text={text} /> : <span className="tk-muted">Nothing to preview yet.</span>}
                </div>
              )}
              {kind === "file" && (
                <div className="nt-file">
                  <button type="button" className={`nt-file-btn ${planErr ? "err" : ""}`} onClick={browse} disabled={!repo}>
                    <span className={`mono ellipsis nt-grow ${path ? "" : "tk-muted"}`}>{path || "Choose a .md file…"}</span>
                    <span className="nt-file-browse">Browse…</span>
                  </button>
                  <span className="tk-hint">Read from the repo each time the task runs.</span>
                </div>
              )}
              {planErr && (
                <span className="nt-err">
                  {kind === "text" ? "Write a plan, or use a .md from the repo." : "Choose a .md file from the repo."}
                </span>
              )}
              {imported && <span className="tk-hint">Saving a new plan overrides the synced description.</span>}
            </section>

            <section className="nt-sec">
              <div className="nt-sec-head nt-sec-head-base">
                <span className="nt-sec-title">Done when</span>
                <span className="nt-sec-sub ellipsis">· the reviewer checks each one</span>
                <span className="nt-count">
                  {critCount ? `${critCount} criteri${critCount === 1 ? "on" : "a"}` : ""}
                </span>
              </div>
              {suggestions.length > 0 && (
                <div className="nt-sugg">
                  <div className="nt-sugg-head">
                    <span>Found {suggestions.length} in your plan</span>
                    <button type="button" className="nt-soft-btn" onClick={() => addCriteria(suggestions)}>
                      Add all
                    </button>
                  </div>
                  <div className="nt-sugg-list">
                    {suggestions.map((sg) => (
                      <button key={sg} type="button" className="nt-sugg-item" onClick={() => addCriteria([sg])}>
                        <span className="nt-plus" aria-hidden>
                          +{" "}
                        </span>
                        {sg}
                      </button>
                    ))}
                  </div>
                </div>
              )}
              <div className="nt-crits">
                {acceptance.map((c, i) => {
                  const vague = isVague(c);
                  return (
                    <div key={i} className="nt-crit">
                      <div className="nt-crit-row">
                        <span className={`nt-box ${vague ? "warn" : ""}`} aria-hidden />
                        <input
                          ref={(el) => {
                            acRefs.current[i] = el;
                          }}
                          className="nt-crit-input"
                          value={c}
                          aria-label={`Criterion ${i + 1}`}
                          onChange={(e) => setAc(i, e.target.value)}
                          onKeyDown={(e) => {
                            if (e.key === "Enter" && !e.metaKey) e.currentTarget.blur();
                            else if (e.key === "Backspace" && c === "") {
                              e.preventDefault();
                              removeAc(i);
                            }
                          }}
                        />
                        <button
                          type="button"
                          className="nt-x"
                          aria-label={`Remove criterion ${i + 1}`}
                          onClick={() => removeAc(i)}
                        >
                          ✕
                        </button>
                      </div>
                      {vague && (
                        <div className="nt-vague">Hard to verify. Say what the reviewer would observe: a value, a test, a file.</div>
                      )}
                    </div>
                  );
                })}
                <div className="nt-crit-row nt-crit-new">
                  <span className="nt-box-plus" aria-hidden>
                    +
                  </span>
                  <input
                    ref={draftRef}
                    className="nt-crit-input"
                    value={critDraft}
                    onChange={(e) => setCritDraft(e.target.value)}
                    onKeyDown={onDraftKey}
                    onPaste={onDraftPaste}
                    placeholder="Observable outcome, e.g. retries stop after 5 attempts"
                    aria-label="New criterion"
                  />
                  <span className="kbd" aria-hidden>
                    ↵ add
                  </span>
                </div>
              </div>
              {critCount === 0 && (
                <span className="tk-hint">
                  Without criteria the reviewer only checks the plan steps. Paste a list and each line becomes one criterion.
                </span>
              )}
            </section>

            <section className="nt-exec">
              <button
                type="button"
                className="nt-exec-head"
                aria-expanded={execOpen}
                onClick={() => setExecOpen(!execOpen)}
              >
                <span className="nt-sec-title">Execution</span>
                <span className="nt-exec-sum ellipsis">{execSummary}</span>
                {execCustom && <span className="nt-custom">custom</span>}
                <span className="nt-exec-btn">{execOpen ? "Done" : "Change"}</span>
              </button>
              {execOpen && (
                <div className="nt-exec-body">
                  <span className="nt-exec-label">Executor</span>
                  <div className="nt-exec-opt">
                    <ExecutorPicker
                      repoId={repo?.id ?? null}
                      value={assignee}
                      inherited={inherited}
                      onChange={(v) => setAssignee(v && sameExecutor(v, inherited) ? null : v)}
                      dropUp
                    />
                  </div>

                  <span className="nt-exec-label">Isolation</span>
                  <div className="nt-exec-opt">
                    <Seg
                      label="Isolation"
                      options={ISOLATIONS.map((i) => [i, ISOLATION_LABEL[i]])}
                      value={effIsolation}
                      disabled={isWorkflow}
                      onPick={(v) => choose(v, repo?.defaultIsolation, setIsolation)}
                    />
                    <Hint custom={!isWorkflow && isolation !== null}>
                      {isWorkflow
                        ? "Workflows manage their own worktree."
                        : [ISOLATION_HINT[effIsolation], repoDefault(isolation !== null)].filter(Boolean).join(" · ")}
                    </Hint>
                  </div>

                  <span className="nt-exec-label">Finish</span>
                  <div className="nt-exec-opt">
                    <Seg
                      label="Finish"
                      options={FINISHES.map((f) => [f, FINISH_LABEL[f]])}
                      value={effFinish}
                      onPick={(v) => choose(v, repo?.defaultFinish, setFinish)}
                    />
                    <Hint custom={finish !== null}>
                      {[FINISH_HINT[effFinish], repoDefault(finish !== null)].filter(Boolean).join(" · ")}
                    </Hint>
                  </div>

                  <span className="nt-exec-label">Review</span>
                  <div className="nt-exec-opt">
                    <button
                      type="button"
                      role="switch"
                      aria-checked={effReview}
                      className="nt-toggle"
                      onClick={() => choose(!effReview, repo?.defaultReview, setReview)}
                    >
                      <span className={`nt-track ${effReview ? "on" : ""}`} aria-hidden>
                        <span className="nt-knob" />
                      </span>
                      Review before In Review
                    </button>
                    <Hint custom={review !== null}>
                      {[
                        effReview
                          ? `${repo?.reviewer ?? project?.reviewer ?? settings.data?.reviewer ?? "code-reviewer"} checks the criteria first, read-only`
                          : "Goes straight to In Review",
                        repoDefault(review !== null),
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </Hint>
                  </div>
                </div>
              )}
            </section>

            {error && (
              <div className="banner banner-error" role="alert">
                {error}
              </div>
            )}
          </div>
        )}

        <footer className="dlg-foot">
          <span className={`dlg-foot-note ${tried && missing.length ? "nt-err" : "tk-hint"}`}>{footNote}</span>
          <button type="button" className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          {!original && (
            <button type="button" className="btn" disabled={saving || !loaded} onClick={() => void submit(true)}>
              Create and run
            </button>
          )}
          <button type="submit" className="btn btn-primary nt-save" disabled={saving || !loaded}>
            {original ? "Save" : "Save as to-do"}
            <span className="nt-kbd" aria-hidden>
              ⌘↵
            </span>
          </button>
        </footer>
      </form>
    </>
  );
}

function Hint({ custom, children }: { custom: boolean; children: string }) {
  return <span className={`tk-hint ${custom ? "nt-hint-custom" : ""}`}>{children}</span>;
}

function Seg<T extends string>({
  label,
  options,
  value,
  disabled,
  onPick,
}: {
  label: string;
  options: [T, string][];
  value: T;
  disabled?: boolean;
  onPick: (v: T) => void;
}) {
  return (
    <div className="segmented tk-seg" role="radiogroup" aria-label={label}>
      {options.map(([v, l]) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={value === v}
          disabled={disabled}
          className={`tk-seg-opt ${value === v ? "on" : ""}`}
          onClick={() => onPick(v)}
        >
          {l}
        </button>
      ))}
    </div>
  );
}
