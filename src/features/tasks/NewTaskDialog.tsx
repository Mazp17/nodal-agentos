import { useEffect, useId, useMemo, useRef, useState, type FormEvent, type KeyboardEvent, type ReactNode } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { createTask, getTask, readTaskPlan, updateTask, type NewTask, type PlanInput, type TaskPatch } from "../../domain/api";
import { invalidate, useProjects, useRepos } from "../../domain/hooks/tasks";
import { taskKey, type Executor, type Finish, type Isolation, type Priority, type Task } from "../../domain/types";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useToast } from "../../ui/Toasts";
import { ExecutorPicker, executorLabel, inheritedExecutor, sameExecutor } from "../executors";
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
  /** Proyecto de la vista; `null` (All projects) pide elegirlo. Al editar manda el de la tarea. */
  projectId: string | null;
  /** Si viene, edita esa tarea. */
  taskId?: string;
  /** Repo preseleccionado (p. ej. el filtro activo del board). */
  defaultRepoId?: string | null;
  onClose: () => void;
  onSaved: (task: Task) => void;
  /** Si viene, muestra "Add repo…" (lo resuelve Settings del proyecto). */
  onAddRepo?: (projectId: string) => void;
}

type PlanKind = PlanInput["kind"];

const FINISHES: Finish[] = ["changes", "commit", "pr"];
const ISOLATIONS: Isolation[] = ["worktree", "in_place"];
const PRIO_ORDER: Priority[] = PRIORITIES.filter((p) => p !== "none");

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const cleanList = (l: string[]) => l.map((s) => s.trim()).filter(Boolean);

/** Crear o editar una tarea: repo obligatorio, plan, criterios, ejecutor y opciones de ejecución. */
export function NewTaskDialog({ projectId, taskId, defaultRepoId, onClose, onSaved, onAddRepo }: NewTaskDialogProps) {
  const ref = useFocusTrap<HTMLFormElement>(onClose);
  const titleId = useId();
  const push = useToast();
  const launch = useLaunch();
  const projects = useProjects();
  const allRepos = useRepos(null);

  const [original, setOriginal] = useState<Task | null>(null);
  const [loaded, setLoaded] = useState(!taskId);
  const [pid, setPid] = useState<string | null>(projectId);
  const [repoId, setRepoId] = useState<string | null>(defaultRepoId ?? null);
  const [title, setTitle] = useState("");
  const [kind, setKind] = useState<PlanKind>("text");
  const [text, setText] = useState("");
  const [origText, setOrigText] = useState("");
  const [path, setPath] = useState("");
  const [priority, setPriority] = useState<Priority>("none");
  const [labels, setLabels] = useState<string[]>([]);
  const [labelDraft, setLabelDraft] = useState("");
  const [acceptance, setAcceptance] = useState<string[]>([]);
  const [assignee, setAssignee] = useState<Executor | null>(null);
  const [isolation, setIsolation] = useState<Isolation | null>(null);
  const [finish, setFinish] = useState<Finish | null>(null);
  const [review, setReview] = useState<boolean | null>(null);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const acRefs = useRef<(HTMLInputElement | null)[]>([]);
  const [focusAc, setFocusAc] = useState<number | null>(null);

  // Editar: precarga la tarea y, si el plan es texto, su contenido.
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

  // Sin proyecto (All projects): el primero.
  useEffect(() => {
    if (!pid && projects.data?.length) setPid(projects.data[0].id);
  }, [pid, projects.data]);

  const project = projects.data?.find((p) => p.id === pid) ?? null;
  const repos = useMemo(
    () => (allRepos.data ?? []).filter((r) => r.projectId === pid).sort((a, b) => a.position - b.position),
    [allRepos.data, pid],
  );
  const repo = repos.find((r) => r.id === repoId) ?? null;

  // Repo por defecto: el pedido o el primero del proyecto.
  useEffect(() => {
    if (!loaded || taskId) return;
    if (repoId && repos.some((r) => r.id === repoId)) return;
    if (repos.length) setRepoId(repos[0].id);
    else if (allRepos.data) setRepoId(null);
  }, [loaded, taskId, repoId, repos, allRepos.data]);

  useEffect(() => {
    if (focusAc === null) return;
    acRefs.current[focusAc]?.focus();
    setFocusAc(null);
  }, [focusAc]);

  const imported = !!original?.source;
  const inherited = inheritedExecutor(repo, project);
  const effExecutor = assignee ?? inherited;
  const isWorkflow = effExecutor.kind === "workflow";
  const effIsolation = isolation ?? repo?.defaultIsolation ?? "worktree";
  const effFinish = finish ?? repo?.defaultFinish ?? "pr";
  const effReview = review ?? repo?.defaultReview ?? true;

  const pickRepo = (id: string) => {
    if (id === repoId) return;
    setRepoId(id);
    // El `.md` es relativo al repo anterior.
    if (kind === "file") setPath("");
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
    const parts = cleanList(labelDraft.split(","));
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

  const setAc = (i: number, v: string) => setAcceptance((a) => a.map((x, k) => (k === i ? v : x)));
  const addAc = (at = acceptance.length) => {
    setAcceptance((a) => [...a.slice(0, at), "", ...a.slice(at)]);
    setFocusAc(at);
  };
  const removeAc = (i: number) => {
    setAcceptance((a) => a.filter((_, k) => k !== i));
    setFocusAc(Math.max(0, i - 1));
  };

  /** `null` = heredar: elegir el mismo valor que el default del repo vuelve a heredar. */
  const choose = <T,>(value: T, def: T | undefined, set: (v: T | null) => void) => set(value === def ? null : value);

  const validate = (): string | null => {
    if (!pid) return "Choose a project.";
    if (!repo) return "Choose a repo from this project.";
    if (!title.trim()) return "Give the task a title.";
    if (kind === "text" && !text.trim()) return "Write a plan in markdown.";
    if (kind === "file" && !path.trim()) return "Choose a .md file from the repo.";
    return null;
  };

  const planInput = (): PlanInput => (kind === "text" ? { kind: "text", text } : { kind: "file", path: path.trim() });

  const submit = async (run: boolean) => {
    const err = validate();
    if (err) {
      setError(err);
      return;
    }
    if (saving || !repo || !pid) return;
    setSaving(true);
    setError(null);
    const ac = cleanList(acceptance);
    const pendingLabel = cleanList(labelDraft.split(",")).filter((l) => !labels.includes(l));
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
      invalidate("tasks");
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

  const noRepos = allRepos.data !== undefined && pid !== null && repos.length === 0;

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
      >
        <header className="dlg-head">
          <h2 id={titleId} className="dlg-title">
            {original ? "Edit task" : "New task"}
          </h2>
          {project && (
            <span className="dlg-proj">
              <span className="dlg-proj-dot" style={{ background: project.color }} aria-hidden />
              {project.name}
            </span>
          )}
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
          <div className="dlg-body">
            {projectId === null && !original && (projects.data?.length ?? 0) > 1 && (
              <Field label="Project">
                <div className="tk-pills" role="radiogroup" aria-label="Project">
                  {projects.data?.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      role="radio"
                      aria-checked={p.id === pid}
                      className={`tk-pill ${p.id === pid ? "on" : ""}`}
                      onClick={() => {
                        setPid(p.id);
                        setRepoId(null);
                        setPath("");
                      }}
                    >
                      <span className="dlg-proj-dot" style={{ background: p.color }} aria-hidden />
                      {p.name}
                    </button>
                  ))}
                </div>
              </Field>
            )}

            <Field label="Repo" hint="required">
              {noRepos ? (
                <div className="tk-inline-note">
                  <span>This project has no repos yet.</span>
                  {onAddRepo && pid && (
                    <button type="button" className="btn btn-sm" onClick={() => onAddRepo(pid)}>
                      Add repo…
                    </button>
                  )}
                </div>
              ) : (
                <div className="tk-pills" role="radiogroup" aria-label="Repo">
                  {repos.map((r) => (
                    <button
                      key={r.id}
                      type="button"
                      role="radio"
                      aria-checked={r.id === repoId}
                      title={r.path}
                      className={`tk-pill ${r.id === repoId ? "on" : ""}`}
                      onClick={() => pickRepo(r.id)}
                    >
                      {r.name}
                    </button>
                  ))}
                  {onAddRepo && pid && (
                    <button type="button" className="tk-pill tk-pill-dashed" onClick={() => onAddRepo(pid)}>
                      Add repo…
                    </button>
                  )}
                </div>
              )}
            </Field>

            <Field label="Title" hint={imported ? `from ${providerLabel(original?.source?.provider ?? "")}` : undefined}>
              <input
                className="input"
                data-autofocus
                value={title}
                disabled={imported}
                onChange={(e) => setTitle(e.target.value)}
                placeholder="e.g. Cache launch-blocker lookups per session"
                aria-label="Title"
              />
            </Field>

            <Field
              label="Plan"
              aside={
                <div className="segmented tk-seg" role="radiogroup" aria-label="Plan source">
                  {(
                    [
                      ["text", "Write markdown"],
                      ["file", "Pick .md from repo"],
                    ] as const
                  ).map(([k, l]) => (
                    <button
                      key={k}
                      type="button"
                      role="radio"
                      aria-checked={kind === k}
                      className={`tk-seg-opt ${kind === k ? "on" : ""}`}
                      onClick={() => setKind(k)}
                    >
                      {l}
                    </button>
                  ))}
                </div>
              }
            >
              {kind === "text" ? (
                <textarea
                  className="textarea tk-plan"
                  value={text}
                  onChange={(e) => setText(e.target.value)}
                  placeholder="## Goal · ## Steps · ## Done when"
                  aria-label="Plan (markdown)"
                  rows={7}
                />
              ) : (
                <div className="tk-file">
                  <button type="button" className="tk-file-btn" onClick={browse} disabled={!repo}>
                    <span className={`mono ellipsis ${path ? "" : "tk-muted"}`}>{path || "Choose a .md file…"}</span>
                    <span className="tk-file-browse">Browse…</span>
                  </button>
                  <span className="tk-hint">Stored relative to the repo and read at launch time.</span>
                </div>
              )}
              {imported && <span className="tk-hint">Saving a new plan overrides the synced description.</span>}
            </Field>

            <Field label="Acceptance criteria" hint="the reviewer checks these">
              <div className="tk-ac-list">
                {acceptance.map((a, i) => (
                  <div key={i} className="tk-ac-row">
                    <span className="tk-ac-bullet" aria-hidden>
                      {i + 1}.
                    </span>
                    <input
                      ref={(el) => {
                        acRefs.current[i] = el;
                      }}
                      className="input"
                      value={a}
                      aria-label={`Criterion ${i + 1}`}
                      placeholder="Observable outcome, e.g. retries stop after 5 attempts"
                      onChange={(e) => setAc(i, e.target.value)}
                      onKeyDown={(e) => {
                        if (e.key === "Enter") {
                          e.preventDefault();
                          addAc(i + 1);
                        } else if (e.key === "Backspace" && a === "") {
                          e.preventDefault();
                          removeAc(i);
                        }
                      }}
                    />
                    <button type="button" className="icon-btn" aria-label={`Remove criterion ${i + 1}`} onClick={() => removeAc(i)}>
                      ✕
                    </button>
                  </div>
                ))}
                <button type="button" className="btn btn-ghost btn-sm tk-ac-add" onClick={() => addAc()}>
                  + Add criterion
                </button>
                {acceptance.length === 0 && (
                  <span className="tk-hint">Empty: the reviewer infers them from the plan and says so.</span>
                )}
              </div>
            </Field>

            <div className="tk-grid">
              <Field label="Priority">
                <div className="segmented tk-seg" role="radiogroup" aria-label="Priority">
                  {PRIO_ORDER.map((p) => (
                    <button
                      key={p}
                      type="button"
                      role="radio"
                      aria-checked={priority === p}
                      disabled={imported}
                      className={`tk-seg-opt ${priority === p ? "on" : ""}`}
                      onClick={() => setPriority(priority === p ? "none" : p)}
                    >
                      {PRIORITY_LABEL[p]}
                    </button>
                  ))}
                </div>
              </Field>
              <Field label="Labels">
                <div className="tk-labels">
                  {labels.map((l) => (
                    <span key={l} className="tk-label">
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
                      className="tk-label-input"
                      value={labelDraft}
                      onChange={(e) => setLabelDraft(e.target.value)}
                      onKeyDown={onLabelKey}
                      onBlur={addLabel}
                      placeholder={labels.length ? "" : "Add labels"}
                      aria-label="Add label"
                    />
                  )}
                </div>
              </Field>
            </div>

            <div className="tk-exec">
              <span className="section-label">Execution</span>
              <div className="tk-exec-grid">
                <span className="tk-exec-label">Executor</span>
                <div>
                  <ExecutorPicker
                    repoId={repo?.id ?? null}
                    value={assignee}
                    inherited={inherited}
                    onChange={(v) => setAssignee(v && sameExecutor(v, inherited) ? null : v)}
                    dropUp
                  />
                </div>

                <span className="tk-exec-label">Isolation</span>
                <div className="tk-exec-opt">
                  <Seg
                    label="Isolation"
                    options={ISOLATIONS.map((i) => [i, ISOLATION_LABEL[i]])}
                    value={effIsolation}
                    disabled={isWorkflow}
                    onPick={(v) => choose(v, repo?.defaultIsolation, setIsolation)}
                  />
                  <span className="tk-hint">
                    {isWorkflow ? "Workflows manage their own worktree." : ISOLATION_HINT[effIsolation]}
                    {isolation === null && repo && !isWorkflow ? " · repo default" : ""}
                  </span>
                </div>

                <span className="tk-exec-label">Finish</span>
                <div className="tk-exec-opt">
                  <Seg
                    label="Finish"
                    options={FINISHES.map((f) => [f, FINISH_LABEL[f]])}
                    value={effFinish}
                    onPick={(v) => choose(v, repo?.defaultFinish, setFinish)}
                  />
                  <span className="tk-hint">
                    {FINISH_HINT[effFinish]}
                    {finish === null && repo ? " · repo default" : ""}
                  </span>
                </div>

                <span className="tk-exec-label">Review</span>
                <div className="tk-exec-opt">
                  <label className="tk-switch">
                    <input
                      type="checkbox"
                      role="switch"
                      checked={effReview}
                      onChange={(e) => choose(e.target.checked, repo?.defaultReview, setReview)}
                    />
                    <span>{effReview ? "Review before In Review" : "No review"}</span>
                  </label>
                  <span className="tk-hint">
                    {effReview
                      ? `${repo?.reviewer ?? project?.reviewer ?? "code-reviewer"} checks the criteria, read-only.`
                      : "The task goes to In Review when the run ends."}
                    {review === null && repo ? " · repo default" : ""}
                  </span>
                </div>
              </div>
            </div>

            {error && (
              <div className="banner banner-error" role="alert">
                {error}
              </div>
            )}
          </div>
        )}

        <footer className="dlg-foot">
          <span className="tk-hint dlg-foot-note">
            {repo ? `Runs with ${executorLabel(effExecutor)}${assignee ? "" : " by default"}` : ""}
          </span>
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className={`btn ${original ? "btn-primary" : ""}`} disabled={saving || !loaded}>
            {original ? "Save" : "Create"}
          </button>
          {!original && (
            <button type="button" className="btn btn-primary" disabled={saving || !loaded} onClick={() => void submit(true)}>
              Create and run
            </button>
          )}
        </footer>
      </form>
    </>
  );
}

function Field({
  label,
  hint,
  aside,
  children,
}: {
  label: string;
  hint?: string;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <div className="tk-field">
      <div className="tk-field-head">
        <span className="section-label">
          {label}
          {hint && <span className="tk-field-hint"> · {hint}</span>}
        </span>
        {aside}
      </div>
      {children}
    </div>
  );
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
