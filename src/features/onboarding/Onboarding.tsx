import { useEffect, useId, useRef, useState } from "react";
import { isValidProjectKey, suggestProjectKey, useProjects } from "../../domain/hooks/projects";
import type { Project, ScopeRef } from "../../domain/types";
import { createdSummary, createProjectWithRepos, PROJECT_COLORS } from "../projects/create";
import { ColorSwatches, LinearTeamPicker } from "../projects/fields";
import { DraftRepoRow, RepoNoticeBar, useRepoPicker } from "../projects/repoPicker";
import { isConnected, useProviderStatus } from "../../shell/providerStatus";
import { BrandMark } from "../../ui/BrandMark";
import { useToast } from "../../ui/Toasts";
import "./onboarding.css";

type Step = "project" | "repos" | "source";
const STEPS: { id: Step; label: string }[] = [
  { id: "project", label: "Project" },
  { id: "repos", label: "Repos" },
  { id: "source", label: "Task manager" },
];

interface Props {
  onDone: (p: Project) => void;
  /** Solo si ya hay proyectos (se abrió desde la paleta). */
  onCancel?: () => void;
  /** "Import data from a previous version…" (Settings → Diagnostics). */
  onImportLegacy: () => void;
  importingLegacy?: boolean;
}

export function Onboarding({ onDone, onCancel, onImportLegacy, importingLegacy }: Props) {
  const ctx = useProjects();
  const toast = useToast();
  const provider = useProviderStatus();
  const nameId = useId();
  const [step, setStep] = useState<Step>("project");
  const [name, setName] = useState("");
  const [color, setColor] = useState<string>(PROJECT_COLORS[ctx.projects.length % PROJECT_COLORS.length]!);
  const [repos, setRepos] = useState<string[]>([]);
  const [scope, setScope] = useState<ScopeRef | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const picker = useRepoPicker({
    whereAdded: (root) => {
      if (repos.includes(root)) return "";
      const r = ctx.repos.find((x) => x.path === root);
      return r ? (ctx.projectById.get(r.projectId)?.name ?? "another project") : null;
    },
    onPicked: (root) => setRepos((rs) => [...rs, root]),
  });

  const idx = STEPS.findIndex((s) => s.id === step);

  // Al cambiar de paso el botón con foco se desmonta: el foco va al título del paso nuevo.
  const headingRef = useRef<HTMLHeadingElement>(null);
  const firstStep = useRef(true);
  useEffect(() => {
    if (firstStep.current) {
      firstStep.current = false;
      return;
    }
    headingRef.current?.focus();
  }, [step]);
  const linearOk = isConnected(provider.linear);

  const next1 = () => {
    if (!name.trim()) {
      setMsg("Name your project.");
      return;
    }
    setMsg(null);
    setStep("repos");
  };

  const finish = async (withSource: boolean) => {
    const key = suggestProjectKey(name, ctx.projects.map((p) => p.key));
    if (!isValidProjectKey(key)) return;
    setSaving(true);
    try {
      const out = await createProjectWithRepos(ctx, {
        name,
        key,
        color,
        repos,
        scope: withSource && linearOk ? scope : null,
        autoKey: true,
      });
      toast("Project created", createdSummary(out.project.name, out.reposAdded, out.sourceConnected), "ok");
      if (out.failures.length) toast("Some steps failed", out.failures.join("\n"), "danger");
      onDone(out.project);
    } catch (e) {
      toast("Couldn't create the project", String(e), "danger");
      setSaving(false);
    }
  };

  return (
    <div className="onboarding" role="main" aria-label="Welcome to Nodal">
      <div className="onboarding-scroll">
        <div className="onboarding-col">
          <div className="onboarding-hero">
            <BrandMark size={40} />
            <div className="onboarding-hero-text">
              <h1 className="onboarding-title">Welcome to Nodal</h1>
              <p className="onboarding-sub">Agent OS for Claude. Plan work, launch Claude Code on your repos, watch every agent.</p>
            </div>
          </div>

          <ol className="onboarding-steps" aria-label="Steps">
            {STEPS.map((s, i) => (
              <li key={s.id} className={i <= idx ? "on" : ""} aria-current={i === idx ? "step" : undefined}>
                <span className="onboarding-bar" aria-hidden />
                <span>{s.label}</span>
              </li>
            ))}
          </ol>

          {step === "project" && (
            <form
              className="panel onboarding-card"
              onSubmit={(e) => {
                e.preventDefault();
                next1();
              }}
            >
              <div className="onboarding-card-head">
                <h2 ref={headingRef} tabIndex={-1} className="onboarding-card-title">
                  Create your first project
                </h2>
                <p className="onboarding-card-text">
                  A project groups the repos and tasks for one product or area. You can add more later.
                </p>
              </div>
              <div className="field">
                <label className="field-label" htmlFor={nameId}>
                  Name
                </label>
                <input
                  id={nameId}
                  autoFocus
                  className="input"
                  placeholder="e.g. Payments"
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    setMsg(null);
                  }}
                />
              </div>
              <div className="field">
                <span className="field-label">Color</span>
                <ColorSwatches value={color} onChange={setColor} />
              </div>
              {msg && (
                <span className="field-error" role="alert">
                  {msg}
                </span>
              )}
              <div className="onboarding-actions">
                <button type="button" className="btn btn-ghost" disabled={importingLegacy} onClick={onImportLegacy}>
                  {importingLegacy ? "Importing…" : "Import data from a previous version…"}
                </button>
                <span className="spacer" />
                {onCancel && (
                  <button type="button" className="btn btn-ghost" onClick={onCancel}>
                    Cancel
                  </button>
                )}
                <button type="submit" className="btn btn-primary">
                  Continue
                </button>
              </div>
            </form>
          )}

          {step === "repos" && (
            <div className="panel onboarding-card">
              <div className="onboarding-card-head">
                <h2 ref={headingRef} tabIndex={-1} className="onboarding-card-title">
                  <span className="project-dot" style={{ ["--project-color" as string]: color }} aria-hidden />
                  Add repos to {name.trim() || "Untitled project"}
                </h2>
                <p className="onboarding-card-text">
                  Pick the root of each git checkout. Claude Code runs there; nothing leaves your Mac.
                </p>
              </div>
              <div className="onboarding-repos">
                {repos.map((r) => (
                  <DraftRepoRow key={r} path={r} onRemove={() => setRepos((rs) => rs.filter((x) => x !== r))} />
                ))}
                {repos.length === 0 && <div className="onboarding-empty">No repos yet</div>}
              </div>
              {picker.notice && (
                <RepoNoticeBar notice={picker.notice} onUseRoot={() => void picker.useRoot()} onDismiss={picker.dismiss} />
              )}
              <div className="onboarding-actions">
                <button type="button" className="btn" disabled={picker.busy} onClick={() => void picker.pick()}>
                  Add repo…
                </button>
                <span className="spacer" />
                <button type="button" className="btn btn-ghost" onClick={() => setStep("project")}>
                  Back
                </button>
                <button type="button" className="btn btn-primary" onClick={() => setStep("source")}>
                  {repos.length ? "Continue" : "Skip for now"}
                </button>
              </div>
            </div>
          )}

          {step === "source" && (
            <div className="panel onboarding-card">
              <div className="onboarding-card-head">
                <h2 ref={headingRef} tabIndex={-1} className="onboarding-card-title">
                  Connect a task manager <span className="onboarding-optional">· optional</span>
                </h2>
                <p className="onboarding-card-text">
                  Import issues as tasks and keep their state in sync. Nodal works fully without one.
                </p>
              </div>
              <div className="onboarding-provider">
                <div className="onboarding-provider-head">
                  <span className="provider-mark" aria-hidden />
                  <span className="onboarding-provider-name">Linear</span>
                </div>
                <LinearTeamPicker value={scope} onChange={setScope} allowKey />
              </div>
              <div className="onboarding-actions">
                <span className="faint">Asana, Azure DevOps · coming soon</span>
                <span className="spacer" />
                <button type="button" className="btn btn-ghost" disabled={saving} onClick={() => setStep("repos")}>
                  Back
                </button>
                <button type="button" className="btn" disabled={saving} onClick={() => void finish(false)}>
                  Skip
                </button>
                {linearOk && (
                  <button
                    type="button"
                    className="btn btn-primary"
                    disabled={saving || !scope}
                    onClick={() => void finish(true)}
                  >
                    {saving ? "Creating…" : "Create project"}
                  </button>
                )}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
