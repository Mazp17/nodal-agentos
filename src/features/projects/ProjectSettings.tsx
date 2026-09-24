import { useEffect, useId, useState, type ReactNode } from "react";
import { useProjects } from "../../domain/hooks/projects";
import type { Finish, Isolation, Project, Repo } from "../../domain/types";
import { ProjectSourcesSettings } from "../providers";
import { SectionHead } from "../settings/SettingsView";
import type { ProjectSection } from "../../shell/useNav";
import { useToast } from "../../ui/Toasts";
import { ExecutorSelect } from "./executors";
import { ColorSwatches, Segmented, type SegOption } from "./fields";
import { RepoNoticeBar, resolveGitRoot, useRepoPicker } from "./repoPicker";
import "./projects.css";

const SECTIONS: { id: ProjectSection; label: string }[] = [
  { id: "general", label: "General" },
  { id: "repos", label: "Repos" },
  { id: "sources", label: "Sources" },
];

interface Props {
  project: Project;
  section: ProjectSection;
  onSection: (s: ProjectSection) => void;
  onDeleted: () => void;
}

export function ProjectSettings({ project, section, onSection, onDeleted }: Props) {
  return (
    <div className="settings">
      <nav className="settings-nav" aria-label="Project settings sections">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            type="button"
            className={`settings-nav-item ${section === s.id ? "on" : ""}`}
            aria-current={section === s.id ? "page" : undefined}
            onClick={() => onSection(s.id)}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <div className="settings-scroll">
        <div className="settings-col">
          {section === "general" && <GeneralSection key={project.id} project={project} onDeleted={onDeleted} />}
          {section === "repos" && <ReposSection key={project.id} project={project} />}
          {section === "sources" && <ProjectSourcesSettings projectId={project.id} />}
        </div>
      </div>
    </div>
  );
}

// ---------- General ----------

function GeneralSection({ project, onDeleted }: { project: Project; onDeleted: () => void }) {
  const ctx = useProjects();
  const toast = useToast();
  const nameId = useId();
  const execId = useId();
  const reviewerId = useId();
  const [name, setName] = useState(project.name);
  const [reviewer, setReviewer] = useState(project.reviewer ?? "");
  const [confirming, setConfirming] = useState(false);
  const [deleting, setDeleting] = useState(false);

  useEffect(() => setName(project.name), [project.name]);
  useEffect(() => setReviewer(project.reviewer ?? ""), [project.reviewer]);

  const patch = async (p: Parameters<typeof ctx.updateProject>[1]) => {
    try {
      await ctx.updateProject(project.id, p);
    } catch (e) {
      toast("Couldn't save the project", String(e), "danger");
      setName(project.name);
      setReviewer(project.reviewer ?? "");
    }
  };

  const commitName = () => {
    const n = name.trim();
    if (!n) setName(project.name);
    else if (n !== project.name) void patch({ name: n });
  };
  const commitReviewer = () => {
    const r = reviewer.trim();
    if ((r || null) !== project.reviewer) void patch({ reviewer: r || null });
  };

  const remove = async () => {
    setDeleting(true);
    try {
      await ctx.deleteProject(project.id);
      toast("Project deleted", `${project.name} · repos stay on disk`, "ok");
      onDeleted();
    } catch (e) {
      toast("Couldn't delete the project", String(e), "danger");
      setDeleting(false);
      setConfirming(false);
    }
  };

  return (
    <>
      <SectionHead title="General" />
      <div className="panel settings-card settings-card-pad">
        <div className="field">
          <label className="field-label" htmlFor={nameId}>
            Name
          </label>
          <input
            id={nameId}
            className="input field-w-md"
            placeholder="Project name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            onBlur={commitName}
            onKeyDown={(e) => e.key === "Enter" && commitName()}
          />
        </div>
        <div className="field">
          <span className="field-label">Color</span>
          <ColorSwatches value={project.color} onChange={(color) => void patch({ color })} />
        </div>
        <div className="field">
          <span className="field-label">Task prefix</span>
          <span className="mono muted">{project.key}-</span>
        </div>
        <div className="field">
          <label className="field-label" htmlFor={execId}>
            Default executor
          </label>
          <ExecutorSelect
            id={execId}
            repoId={null}
            value={project.defaultExecutor}
            inheritLabel="Claude (default)"
            onChange={(defaultExecutor) => void patch({ defaultExecutor })}
          />
          <span className="field-hint">Used when neither the task nor its repo picks one.</span>
        </div>
        <div className="field">
          <label className="field-label" htmlFor={reviewerId}>
            Reviewer
          </label>
          <input
            id={reviewerId}
            className="input input-mono field-w-md"
            placeholder="Global default"
            spellCheck={false}
            value={reviewer}
            onChange={(e) => setReviewer(e.target.value)}
            onBlur={commitReviewer}
            onKeyDown={(e) => e.key === "Enter" && commitReviewer()}
          />
          <span className="field-hint">Agent that reviews finished runs. Empty uses the one in Settings → Execution.</span>
        </div>
      </div>

      <div className="panel settings-card">
        <div className="settings-row">
          <div className="settings-row-text">
            <span className="settings-row-title">Delete project</span>
            <span className="settings-row-hint">
              {confirming
                ? `Deletes ${project.name} with its tasks and sources. Repos stay on disk; linked issues stay in their source. This can't be undone.`
                : "Repos stay on disk. Linked issues stay in their source."}
            </span>
          </div>
          {confirming ? (
            <div className="row-actions">
              <button type="button" className="btn btn-ghost" disabled={deleting} onClick={() => setConfirming(false)}>
                Cancel
              </button>
              <button type="button" className="btn btn-danger" disabled={deleting} onClick={() => void remove()} autoFocus>
                {deleting ? "Deleting…" : `Delete ${project.name}`}
              </button>
            </div>
          ) : (
            <button type="button" className="btn btn-danger" onClick={() => setConfirming(true)}>
              Delete
            </button>
          )}
        </div>
      </div>
    </>
  );
}

// ---------- Repos ----------

function ReposSection({ project }: { project: Project }) {
  const ctx = useProjects();
  const toast = useToast();
  const repos = ctx.reposOf(project.id);
  const [editing, setEditing] = useState<string | null>(null);

  const picker = useRepoPicker({
    whereAdded: (root) => {
      const r = ctx.repos.find((x) => x.path === root);
      return r ? (ctx.projectById.get(r.projectId)?.name ?? "another project") : null;
    },
    onPicked: async (root) => {
      try {
        const r = await ctx.addRepo(project.id, { path: root });
        toast("Repo added", `${r.name} · ${project.name}`, "ok");
      } catch (e) {
        picker.setNotice({ kind: "bad", text: String(e) });
      }
    },
  });

  return (
    <>
      <SectionHead title="Repos" text="Local git checkouts in this project. Each task runs in exactly one of them.">
        <button type="button" className="btn" disabled={picker.busy} onClick={() => void picker.pick()}>
          Add repo…
        </button>
      </SectionHead>
      {picker.notice && (
        <RepoNoticeBar notice={picker.notice} onUseRoot={() => void picker.useRoot()} onDismiss={picker.dismiss} />
      )}
      {repos.map((r) => (
        <RepoCard
          key={r.id}
          repo={r}
          editing={editing === r.id}
          onEdit={() => setEditing(editing === r.id ? null : r.id)}
        />
      ))}
      {repos.length === 0 && (
        <div className="center-state">
          <div className="center-state-body">
            <div className="state-icon-empty" aria-hidden />
            <div className="center-state-title">No repos in this project</div>
            <div className="center-state-text">
              Add the root of a git checkout. If you pick a subfolder, Nodal offers the repo root.
            </div>
            <div className="center-state-actions">
              <button type="button" className="btn btn-primary" disabled={picker.busy} onClick={() => void picker.pick()}>
                Add repo…
              </button>
            </div>
          </div>
        </div>
      )}
    </>
  );
}

const MODELS: SegOption<string | null>[] = [
  { value: null, label: "Default" },
  { value: "haiku", label: "haiku" },
  { value: "sonnet", label: "sonnet" },
  { value: "opus", label: "opus" },
];
const EFFORTS: SegOption<string | null>[] = [
  { value: null, label: "Default" },
  ...["low", "medium", "high", "xhigh", "max"].map((v) => ({ value: v as string | null, label: v })),
];
const PERMS: SegOption<string | null>[] = [
  { value: null, label: "Default" },
  { value: "acceptEdits", label: "acceptEdits" },
  { value: "plan", label: "plan" },
  { value: "auto", label: "auto" },
  { value: "bypassPermissions", label: "bypass" },
];
const FINISHES: SegOption<Finish>[] = [
  { value: "changes", label: "Changes only" },
  { value: "commit", label: "Commit" },
  { value: "pr", label: "Open PR" },
];
const ISOLATIONS: SegOption<Isolation>[] = [
  { value: "worktree", label: "Worktree" },
  { value: "in_place", label: "In place" },
];
const REVIEW: SegOption<boolean>[] = [
  { value: true, label: "On" },
  { value: false, label: "Off" },
];

const FINISH_SUMMARY: Record<Finish, string> = { changes: "changes only", commit: "commit", pr: "open PR" };

function repoSummary(r: Repo): string {
  return [
    r.model ?? "default model",
    `${r.effort ?? "default"} effort`,
    r.permissionMode ?? "default permissions",
    FINISH_SUMMARY[r.defaultFinish],
    r.defaultIsolation === "worktree" ? "worktree" : "in place",
    r.defaultReview ? "review on" : "review off",
  ].join(" · ");
}

function RepoCard({ repo, editing, onEdit }: { repo: Repo; editing: boolean; onEdit: () => void }) {
  const ctx = useProjects();
  const toast = useToast();
  const execId = useId();
  const reviewerId = useId();
  const [git, setGit] = useState<"checking" | "ok" | "missing">("checking");
  const [reviewer, setReviewer] = useState(repo.reviewer ?? "");
  const [confirmRemove, setConfirmRemove] = useState(false);

  useEffect(() => {
    let alive = true;
    resolveGitRoot(repo.path).then(
      (root) => alive && setGit(root ? "ok" : "missing"),
      () => alive && setGit("missing"),
    );
    return () => {
      alive = false;
    };
  }, [repo.path]);
  useEffect(() => setReviewer(repo.reviewer ?? ""), [repo.reviewer]);

  const patch = async (p: Parameters<typeof ctx.updateRepo>[1]) => {
    try {
      await ctx.updateRepo(repo.id, p);
    } catch (e) {
      toast("Couldn't save the repo options", String(e), "danger");
    }
  };
  const commitReviewer = () => {
    const r = reviewer.trim();
    if ((r || null) !== repo.reviewer) void patch({ reviewer: r || null });
  };

  const remove = async () => {
    try {
      await ctx.deleteRepo(repo.id);
      toast("Repo removed", `${repo.name} · the folder stays on disk`, "ok");
    } catch (e) {
      toast("Couldn't remove the repo", String(e), "danger");
      setConfirmRemove(false);
    }
  };

  const gitLabel =
    git === "checking" ? "checking…" : git === "ok" ? "git repository" : "folder missing or not a git repository";

  return (
    <div className="panel repo-card">
      <div className="repo-card-head">
        <span className="repo-card-name">{repo.name}</span>
        <span className={`repo-card-status tone-${git === "ok" ? "ok" : git === "missing" ? "danger" : "muted"}`}>
          <span className="dot dot-sm" aria-hidden />
          {gitLabel}
        </span>
        <span className="spacer" />
        {confirmRemove ? (
          <>
            <span className="faint repo-card-confirm">Remove from {ctx.projectById.get(repo.projectId)?.name ?? "project"}?</span>
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirmRemove(false)}>
              Cancel
            </button>
            <button type="button" className="btn btn-danger btn-sm" onClick={() => void remove()}>
              Remove
            </button>
          </>
        ) : (
          <>
            <button type="button" className="btn btn-sm" aria-expanded={editing} onClick={onEdit}>
              {editing ? "Done" : "Edit options"}
            </button>
            <button type="button" className="btn btn-ghost btn-sm" onClick={() => setConfirmRemove(true)}>
              Remove
            </button>
          </>
        )}
      </div>
      <div className="repo-card-meta">
        <span className="mono repo-card-path" title={repo.path}>
          {repo.path}
        </span>
        <span className="faint">{repoSummary(repo)}</span>
      </div>
      {editing && (
        <div className="repo-opts">
          <OptRow label="Model">
            <Segmented label="Model" value={repo.model ?? null} options={MODELS} onChange={(model) => void patch({ model })} />
          </OptRow>
          <OptRow label="Effort">
            <Segmented label="Effort" value={repo.effort ?? null} options={EFFORTS} onChange={(effort) => void patch({ effort })} />
          </OptRow>
          <OptRow label="Permission mode">
            <Segmented
              label="Permission mode"
              value={repo.permissionMode ?? null}
              options={PERMS}
              onChange={(permissionMode) => void patch({ permissionMode })}
            />
          </OptRow>
          <OptRow label="Finish">
            <Segmented
              label="Finish"
              value={repo.defaultFinish}
              options={FINISHES}
              onChange={(defaultFinish) => void patch({ defaultFinish })}
            />
          </OptRow>
          <OptRow label="Isolation">
            <Segmented
              label="Isolation"
              value={repo.defaultIsolation}
              options={ISOLATIONS}
              onChange={(defaultIsolation) => void patch({ defaultIsolation })}
            />
          </OptRow>
          <OptRow label="Executor" htmlFor={execId}>
            <ExecutorSelect
              id={execId}
              repoId={repo.id}
              value={repo.defaultExecutor}
              inheritLabel="Project default"
              onChange={(defaultExecutor) => void patch({ defaultExecutor })}
            />
          </OptRow>
          <OptRow label="Review">
            <Segmented
              label="Review"
              value={repo.defaultReview}
              options={REVIEW}
              onChange={(defaultReview) => void patch({ defaultReview })}
            />
          </OptRow>
          <OptRow label="Reviewer" htmlFor={reviewerId}>
            <input
              id={reviewerId}
              className="input input-mono repo-opt-input"
              placeholder="Project default"
              spellCheck={false}
              value={reviewer}
              onChange={(e) => setReviewer(e.target.value)}
              onBlur={commitReviewer}
              onKeyDown={(e) => e.key === "Enter" && commitReviewer()}
            />
          </OptRow>
          <p className="field-hint repo-opts-note">
            Workflows manage their own worktree and review; isolation and review apply to agents and Claude.
          </p>
        </div>
      )}
    </div>
  );
}

function OptRow({ label, htmlFor, children }: { label: string; htmlFor?: string; children: ReactNode }) {
  return (
    <div className="repo-opt">
      {htmlFor ? (
        <label className="repo-opt-label" htmlFor={htmlFor}>
          {label}
        </label>
      ) : (
        <span className="repo-opt-label">{label}</span>
      )}
      {children}
    </div>
  );
}
