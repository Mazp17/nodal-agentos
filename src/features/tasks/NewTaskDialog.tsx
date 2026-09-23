import { useEffect, useId, useState, type FormEvent } from "react";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { createTask, readTaskPlan, updateTask } from "./api";
import type { PlanInput, Task } from "./types";
import "./tasks.css";

export interface NewTaskDialogProps {
  /** Repos entre los que elegir (los mapeados en Settings). */
  repos: string[];
  /** Repo preseleccionado (p. ej. el de la vista actual). */
  defaultRepo?: string | null;
  /** Si viene, el diálogo edita esa tarea (el repo no se puede cambiar). */
  task?: Task | null;
  /**
   * Selector nativo de archivos (tauri-plugin-dialog). Recibe el repo elegido para
   * abrir ahí; devuelve la ruta o `null` si se cancela. Sin él, se escribe la ruta.
   */
  pickFile?: (repoPath: string) => Promise<string | null>;
  onClose: () => void;
  onSaved: (task: Task) => void;
}

type PlanKind = PlanInput["kind"];

const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/** Crear o editar una tarea local: repo, título y plan (texto markdown o `.md` del repo). */
export function NewTaskDialog({ repos, defaultRepo, task, pickFile, onClose, onSaved }: NewTaskDialogProps) {
  const ref = useFocusTrap<HTMLFormElement>(onClose);
  const titleId = useId();
  const editing = !!task;
  const [repo, setRepo] = useState(task?.repoPath ?? defaultRepo ?? repos[0] ?? "");
  const [title, setTitle] = useState(task?.title ?? "");
  const [kind, setKind] = useState<PlanKind>(task?.plan.kind ?? "text");
  const [text, setText] = useState("");
  const [path, setPath] = useState(task?.plan.kind === "file" ? task.plan.path : "");
  const [loadingPlan, setLoadingPlan] = useState(task?.plan.kind === "text");
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Al editar un plan en texto, se precarga el contenido actual.
  useEffect(() => {
    if (!task || task.plan.kind !== "text") return;
    let alive = true;
    readTaskPlan(task.id)
      .then((t) => alive && setText(t))
      .catch((e) => alive && setError(String(e)))
      .finally(() => alive && setLoadingPlan(false));
    return () => {
      alive = false;
    };
  }, [task]);

  const browse = async () => {
    if (!pickFile || !repo) return;
    try {
      const picked = await pickFile(repo);
      if (picked) setPath(picked);
    } catch (e) {
      setError(String(e));
    }
  };

  const valid = !!repo && title.trim() !== "" && (kind === "text" ? text.trim() !== "" : path.trim() !== "");

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!valid || saving) return;
    setSaving(true);
    setError(null);
    const plan: PlanInput = kind === "text" ? { kind: "text", text } : { kind: "file", path: path.trim() };
    try {
      const saved = task ? await updateTask(task.id, title, plan) : await createTask(repo, title, plan);
      onSaved(saved);
    } catch (err) {
      setError(String(err));
      setSaving(false);
    }
  };

  // Opciones: los repos mapeados, más el de la tarea si ya no está mapeado.
  const options = repo && !repos.includes(repo) ? [repo, ...repos] : repos;

  return (
    <>
      <div className="tk-dialog-scrim" onClick={onClose} aria-hidden />
      <form
        ref={ref}
        className="tk-dialog"
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onSubmit={submit}
      >
        <header className="tk-dialog-head">
          <h2 id={titleId} className="tk-dialog-title">
            {editing ? "Edit task" : "New task"}
          </h2>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>

        <div className="tk-dialog-body">
          <label className="tk-field">
            <span className="section-label">Repository</span>
            {options.length === 0 ? (
              <span className="tk-hint">No repositories mapped yet. Map one in Settings first.</span>
            ) : (
              <select
                className="input input-mono"
                value={repo}
                disabled={editing}
                onChange={(e) => setRepo(e.target.value)}
              >
                {options.map((r) => (
                  <option key={r} value={r}>
                    {basename(r)} — {r}
                  </option>
                ))}
              </select>
            )}
          </label>

          <label className="tk-field">
            <span className="section-label">Title</span>
            <input
              className="input"
              value={title}
              maxLength={200}
              placeholder="What should the agent do?"
              onChange={(e) => setTitle(e.target.value)}
              data-autofocus
            />
          </label>

          <div className="tk-field">
            <div className="tk-field-row">
              <span className="section-label" id={`${titleId}-plan`}>
                Plan
              </span>
              <div className="tk-seg" role="radiogroup" aria-labelledby={`${titleId}-plan`}>
                {(["text", "file"] as const).map((k) => (
                  <button
                    key={k}
                    type="button"
                    role="radio"
                    aria-checked={kind === k}
                    className={`tk-seg-btn ${kind === k ? "on" : ""}`}
                    onClick={() => setKind(k)}
                  >
                    {k === "text" ? "Write" : "Markdown file"}
                  </button>
                ))}
              </div>
            </div>
            {kind === "text" ? (
              <textarea
                className="textarea tk-plan-input"
                value={text}
                rows={14}
                spellCheck={false}
                disabled={loadingPlan}
                placeholder={loadingPlan ? "Loading plan…" : "# Goal\n\nWhat to change and why.\n\n## Acceptance\n\n- …"}
                aria-label="Plan (Markdown)"
                onChange={(e) => setText(e.target.value)}
              />
            ) : (
              <>
                <div className="tk-file-row">
                  <input
                    className="input input-mono tk-grow"
                    value={path}
                    placeholder="docs/plan.md"
                    aria-label="Plan file path"
                    onChange={(e) => setPath(e.target.value)}
                  />
                  {pickFile && (
                    <button type="button" className="btn" onClick={() => void browse()} disabled={!repo}>
                      Browse…
                    </button>
                  )}
                </div>
                <span className="tk-hint">A .md file inside the repository — relative to its root, or absolute.</span>
              </>
            )}
          </div>

          {error && (
            <div className="banner banner-error" role="alert">
              {error}
            </div>
          )}
        </div>

        <footer className="tk-dialog-foot">
          <button type="button" className="btn" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" className="btn btn-primary" disabled={!valid || saving || loadingPlan}>
            {saving ? "Saving…" : editing ? "Save" : "Create task"}
          </button>
        </footer>
      </form>
    </>
  );
}
