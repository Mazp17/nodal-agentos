import { useId, useState } from "react";
import { isValidProjectKey, suggestProjectKey, useProjects } from "../../domain/hooks/projects";
import type { Project, ScopeRef } from "../../domain/types";
import { useToast } from "../../ui/Toasts";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { createdSummary, createProjectWithRepos, PROJECT_COLORS } from "./create";
import { ColorSwatches, LinearTeamPicker } from "./fields";
import { DraftRepoRow, RepoNoticeBar, useRepoPicker } from "./repoPicker";
import "./projects.css";

interface Props {
  onClose: () => void;
  onCreated: (p: Project) => void;
}

export function CreateProjectDialog({ onClose, onCreated }: Props) {
  const ctx = useProjects();
  const toast = useToast();
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const titleId = useId();
  const nameId = useId();
  const keyId = useId();

  const [name, setName] = useState("");
  const [key, setKey] = useState("");
  const [keyEdited, setKeyEdited] = useState(false);
  const [color, setColor] = useState<string>(PROJECT_COLORS[ctx.projects.length % PROJECT_COLORS.length]!);
  const [repos, setRepos] = useState<string[]>([]);
  const [connect, setConnect] = useState(false);
  const [scope, setScope] = useState<ScopeRef | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const taken = ctx.projects.map((p) => p.key);
  const effectiveKey = keyEdited ? key : suggestProjectKey(name, taken);

  const picker = useRepoPicker({
    whereAdded: (root) => {
      if (repos.includes(root)) return "";
      const r = ctx.repos.find((x) => x.path === root);
      return r ? (ctx.projectById.get(r.projectId)?.name ?? "another project") : null;
    },
    onPicked: (root) => setRepos((rs) => [...rs, root]),
  });

  const create = async () => {
    if (!name.trim()) {
      setError("Name your project.");
      return;
    }
    if (!isValidProjectKey(effectiveKey)) {
      setError("Task prefix: use 2-6 letters or digits, starting with a letter.");
      return;
    }
    setSaving(true);
    setError(null);
    try {
      const { project, reposAdded, sourceConnected, failures } = await createProjectWithRepos(ctx, {
        name,
        key: effectiveKey,
        color,
        repos,
        scope: connect ? scope : null,
      });
      toast("Project created", createdSummary(project.name, reposAdded, sourceConnected), "ok");
      if (failures.length) toast("Some steps failed", failures.join("\n"), "danger");
      onCreated(project);
    } catch (e) {
      setError(String(e));
      setSaving(false);
    }
  };

  return (
    <>
      <div className="dialog-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        <div className="dialog-head">
          <h2 id={titleId} className="dialog-title">
            New project
          </h2>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </div>
        <form
          className="dialog-body"
          id={`${titleId}-form`}
          onSubmit={(e) => {
            e.preventDefault();
            void create();
          }}
        >
          <div className="field">
            <label className="field-label" htmlFor={nameId}>
              Name
            </label>
            <input
              id={nameId}
              data-autofocus
              className="input"
              placeholder="e.g. Billing"
              value={name}
              onChange={(e) => {
                setName(e.target.value);
                setError(null);
              }}
            />
          </div>
          <div className="field">
            <span className="field-label">Color</span>
            <ColorSwatches value={color} onChange={setColor} />
          </div>
          <div className="field">
            <label className="field-label" htmlFor={keyId}>
              Task prefix
            </label>
            <div className="key-row">
              <input
                id={keyId}
                className="input input-mono key-input"
                value={effectiveKey}
                maxLength={6}
                spellCheck={false}
                aria-describedby={`${keyId}-hint`}
                onChange={(e) => {
                  setKeyEdited(true);
                  setKey(e.target.value.toUpperCase().replace(/[^A-Z0-9]/g, ""));
                  setError(null);
                }}
              />
              <span id={`${keyId}-hint`} className="field-hint">
                Task ids look like {effectiveKey || "KEY"}-1.
              </span>
            </div>
          </div>
          <div className="field">
            <span className="field-label">Repos</span>
            {repos.map((r) => (
              <DraftRepoRow key={r} path={r} onRemove={() => setRepos((rs) => rs.filter((x) => x !== r))} />
            ))}
            {picker.notice && (
              <RepoNoticeBar notice={picker.notice} onUseRoot={() => void picker.useRoot()} onDismiss={picker.dismiss} />
            )}
            <button
              type="button"
              className="btn"
              style={{ alignSelf: "flex-start" }}
              disabled={picker.busy}
              onClick={() => void picker.pick()}
            >
              Add repo…
            </button>
          </div>
          <div className="connect-box">
            <div className="connect-box-head">
              <button
                type="button"
                role="switch"
                aria-checked={connect}
                aria-label="Connect a Linear team"
                className="switch"
                onClick={() => setConnect(!connect)}
              />
              <span>Connect a Linear team</span>
              <span className="faint">optional</span>
            </div>
            {connect && <LinearTeamPicker value={scope} onChange={setScope} allowKey={false} />}
          </div>
          {error && (
            <span className="field-error" role="alert">
              {error}
            </span>
          )}
        </form>
        <div className="dialog-foot">
          <span className="spacer" />
          <button type="button" className="btn btn-ghost" onClick={onClose}>
            Cancel
          </button>
          <button type="submit" form={`${titleId}-form`} className="btn btn-primary" disabled={saving}>
            {saving ? "Creating…" : "Create project"}
          </button>
        </div>
      </div>
    </>
  );
}
