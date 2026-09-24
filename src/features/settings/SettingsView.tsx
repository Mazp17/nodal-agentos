import { useCallback, useEffect, useId, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { providerStatus, setSettings, type ProviderStatus } from "../../domain/api";
import { useProjects } from "../../domain/hooks/projects";
import type { Settings } from "../../domain/types";
import { IntegrationsSettings } from "../providers";
import { resolveGitRoot } from "../projects/repoPicker";
import { Segmented } from "../projects/fields";
import { invalidate, KEYS, setData, useSettings } from "../../domain/hooks/store";
import { CLAUDE } from "../executors/executors";
import { ExecutorPicker } from "../executors";
import type { SettingsSection } from "../../shell/useNav";
import { useToast } from "../../ui/Toasts";
import { summarize, useLegacyImport } from "./legacyImport";
import "./settings.css";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "integrations", label: "Integrations" },
  { id: "execution", label: "Execution" },
  { id: "diagnostics", label: "Diagnostics" },
];

interface Props {
  section: SettingsSection;
  onSection: (s: SettingsSection) => void;
}

/** La píldora y los diálogos leen la concurrencia del store compartido. */
function onSettingsSaved(s: Settings) {
  setData<Settings>(KEYS.settings, () => s);
  void invalidate("settings");
}

export function SettingsView({ section, onSection }: Props) {
  return (
    <div className="settings">
      <nav className="settings-nav" aria-label="Settings sections">
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
          {section === "integrations" && <IntegrationsSettings />}
          {section === "execution" && <ExecutionSettings onSaved={onSettingsSaved} />}
          {section === "diagnostics" && <DiagnosticsSettings />}
        </div>
      </div>
    </div>
  );
}

export function SectionHead({ title, text, children }: { title: string; text?: string; children?: ReactNode }) {
  return (
    <div className="settings-head">
      <div className="settings-head-text">
        <h2 className="settings-title">{title}</h2>
        {text && <p className="settings-sub">{text}</p>}
      </div>
      {children}
    </div>
  );
}

// ---------- Execution ----------

/** Valores que acepta el backend (`validate::EDITORS`). */
const EDITORS = ["code", "cursor", "windsurf", "zed", "subl", "idea", "webstorm", "fleet", "code-insiders"] as const;
const EDITOR_LABEL: Record<string, string> = {
  code: "VS Code",
  "code-insiders": "VS Code Insiders",
  cursor: "Cursor",
  windsurf: "Windsurf",
  zed: "Zed",
  subl: "Sublime Text",
  idea: "IntelliJ IDEA",
  webstorm: "WebStorm",
  fleet: "Fleet",
};
const MAX_CONCURRENCY = 16;

function ExecutionSettings({ onSaved }: { onSaved: (s: Settings) => void }) {
  const toast = useToast();
  const settings = useSettings();
  const s = settings.data;
  const [reviewer, setReviewer] = useState("");
  const [saving, setSaving] = useState(false);
  const parId = useId();
  const reviewerId = useId();
  const execId = useId();

  useEffect(() => {
    if (s) setReviewer(s.reviewer);
  }, [s?.reviewer]);

  // Un guardado a la vez (los controles se deshabilitan): sin respuestas desordenadas.
  // Optimista sobre el store compartido; si falla, la relectura lo corrige.
  const save = async (next: Settings) => {
    setData<Settings>(KEYS.settings, () => next);
    setSaving(true);
    try {
      onSaved(await setSettings(next));
    } catch (e) {
      void invalidate("settings");
      toast("Couldn't save settings", String(e), "danger");
    } finally {
      setSaving(false);
    }
  };

  if (!s) {
    return settings.error ? (
      <>
        <SectionHead title="Execution" />
        <div className="field-error" role="alert">
          Couldn't read the settings: {settings.error}
        </div>
      </>
    ) : (
      <SectionHead title="Execution" text="Loading…" />
    );
  }

  const commitReviewer = () => {
    const r = reviewer.trim();
    if (r && r !== s.reviewer) void save({ ...s, reviewer: r });
    else setReviewer(s.reviewer);
  };

  return (
    <>
      <SectionHead title="Execution" text="One global queue for runs across every project." />
      <div className="panel settings-card">
        <div className="settings-row">
          <div className="settings-row-text">
            <span className="settings-row-title" id={parId}>
              Parallel runs
            </span>
            <span className="settings-row-hint">Extra launches wait in the queue.</span>
          </div>
          <div className="stepper" role="group" aria-labelledby={parId}>
            <button
              type="button"
              aria-label="Fewer parallel runs"
              disabled={saving || s.concurrency <= 1}
              onClick={() => void save({ ...s, concurrency: s.concurrency - 1 })}
            >
              −
            </button>
            <span className="stepper-value" aria-live="polite">
              {s.concurrency}
            </span>
            <button
              type="button"
              aria-label="More parallel runs"
              disabled={saving || s.concurrency >= MAX_CONCURRENCY}
              onClick={() => void save({ ...s, concurrency: s.concurrency + 1 })}
            >
              +
            </button>
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-row-text">
            <span className="settings-row-title" id={execId}>
              Default executor
            </span>
            <span className="settings-row-hint">
              Runs tasks when neither the task, its repo nor its project picks one. Projects and repos can override it.
            </span>
          </div>
          <div aria-labelledby={execId} role="group">
            <ExecutorPicker
              repoId={null}
              value={s.defaultExecutor}
              inherited={CLAUDE}
              label="Default executor"
              disabled={saving}
              onChange={(defaultExecutor) => void save({ ...s, defaultExecutor })}
            />
          </div>
        </div>
        <div className="settings-row">
          <div className="settings-row-text">
            <label className="settings-row-title" htmlFor={reviewerId}>
              Default reviewer
            </label>
            <span className="settings-row-hint">
              Agent that checks agent and Claude runs against the acceptance criteria. Projects and repos can override it.
            </span>
          </div>
          <input
            id={reviewerId}
            className="input input-mono settings-input"
            value={reviewer}
            spellCheck={false}
            onChange={(e) => setReviewer(e.target.value)}
            onBlur={commitReviewer}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        </div>
        <div className="settings-row settings-row-top">
          <div className="settings-row-text">
            <span className="settings-row-title">Editor</span>
            <span className="settings-row-hint">Used by "Open in editor" in run diffs.</span>
          </div>
          <Segmented
            label="Editor"
            value={s.editor}
            options={[
              { value: null as string | null, label: "System default" },
              ...EDITORS.map((e) => ({ value: e as string | null, label: EDITOR_LABEL[e] ?? e })),
            ]}
            disabled={saving}
            onChange={(editor) => void save({ ...s, editor })}
          />
        </div>
      </div>
    </>
  );
}

// ---------- Diagnostics ----------

type Check = { name: string; value: string; state: "ok" | "warn" | "error" | "checking" | "na" };

function DiagnosticsSettings() {
  const { repos } = useProjects();
  const legacy = useLegacyImport();
  const [checks, setChecks] = useState<Check[] | null>(null);
  const [busy, setBusy] = useState(false);

  const run = useCallback(async () => {
    setBusy(true);
    const cli = invoke<string>("claude_version").then(
      (v): Check => ({ name: "Claude Code CLI", value: v, state: "ok" }),
      (e): Check => ({ name: "Claude Code CLI", value: String(e), state: "error" }),
    );
    const linear = providerStatus("linear").then(
      (s: ProviderStatus): [Check, Check] => [
        {
          name: "Keychain",
          value: s.hasKey ? "Linear key stored in the macOS Keychain" : "Readable · no keys stored",
          state: "ok",
        },
        s.hasKey
          ? s.error
            ? { name: "Linear", value: s.error, state: "error" }
            : { name: "Linear", value: s.viewer ? `api.linear.app · ${s.viewer}` : "api.linear.app", state: "ok" }
          : { name: "Linear", value: "Not connected", state: "na" },
      ],
      (e): [Check, Check] => [
        { name: "Keychain", value: String(e), state: "error" },
        { name: "Linear", value: "Not checked", state: "na" },
      ],
    );
    // Sin comando propio de git: cada repo se resuelve con `git rev-parse` en el backend.
    const repoChecks = Promise.all(
      repos.map((r) =>
        resolveGitRoot(r.path).then(
          (root) => ({ r, ok: root !== null }),
          () => ({ r, ok: false }),
        ),
      ),
    ).then((rs): [Check, Check] => {
      const bad = rs.filter((x) => !x.ok);
      const git: Check =
        rs.length === 0
          ? { name: "git", value: "Not checked · no repos yet", state: "na" }
          : bad.length === rs.length
            ? { name: "git", value: "Couldn't resolve any repo with git", state: "error" }
            : { name: "git", value: "Resolves repo roots", state: "ok" };
      const reposCheck: Check =
        rs.length === 0
          ? { name: "Repos", value: "No repos yet", state: "na" }
          : bad.length
            ? {
                name: "Repos",
                value: `${rs.length} repos · ${bad.length} missing or not a git repo: ${bad.map((b) => b.r.name).join(", ")}`,
                state: "warn",
              }
            : { name: "Repos", value: `${rs.length} repo${rs.length === 1 ? "" : "s"} · all reachable`, state: "ok" };
      return [git, reposCheck];
    });
    const [c, [kc, lc], [gc, rc]] = await Promise.all([cli, linear, repoChecks]);
    setChecks([c, kc, gc, rc, lc]);
    setBusy(false);
  }, [repos]);

  useEffect(() => {
    void run();
    // Solo al entrar; "Run again" vuelve a correrlo.
  }, []);

  const rows: Check[] =
    busy || !checks
      ? ["Claude Code CLI", "Keychain", "git", "Repos", "Linear"].map((name) => ({ name, value: "", state: "checking" }))
      : checks;

  return (
    <>
      <SectionHead title="Diagnostics">
        <button type="button" className="btn" disabled={busy} onClick={() => void run()}>
          {busy ? "Checking…" : "Run again"}
        </button>
      </SectionHead>
      <div className="panel diag-list" aria-busy={busy}>
        {rows.map((d) => (
          <div key={d.name} className={`diag-row diag-${d.state}`}>
            <span className="diag-mark" aria-hidden>
              {MARK[d.state]}
            </span>
            <span className="diag-name">{d.name}</span>
            <span className="diag-value mono ellipsis" title={d.value}>
              {d.value}
            </span>
            <span className="diag-state">{LABEL[d.state]}</span>
          </div>
        ))}
      </div>
      <p className="settings-note">
        Whether Claude Code trusts each repo (its trust dialog) isn't checked yet.
      </p>

      <SectionHead
        title="Previous version"
        text="Copies the folder you pick to a backup inside Nodal's data folder, then imports its projects, repos, tasks and run history. The original folder isn't modified. Running it twice doesn't duplicate anything."
      />
      <div className="panel settings-card">
        <div className="settings-row">
          <div className="settings-row-text">
            <span className="settings-row-title">Import data from a previous version…</span>
            <span className="settings-row-hint">API keys aren't imported: connect Linear again in Integrations.</span>
          </div>
          <button type="button" className="btn" disabled={legacy.busy} onClick={() => void legacy.run()}>
            {legacy.busy ? "Importing…" : "Choose folder…"}
          </button>
        </div>
        {legacy.report && (
          <div className="legacy-report" role="status">
            <span>{summarize(legacy.report)}</span>
            <span className="mono faint ellipsis" title={legacy.report.backupDir}>
              Backup: {legacy.report.backupDir}
            </span>
            {legacy.report.skipped.length > 0 && (
              <details>
                <summary>Skipped ({legacy.report.skipped.length})</summary>
                <ul>
                  {legacy.report.skipped.map((s, i) => (
                    <li key={i}>{s}</li>
                  ))}
                </ul>
              </details>
            )}
          </div>
        )}
        {legacy.error && (
          <div className="field-error legacy-error" role="alert">
            {legacy.error}
          </div>
        )}
      </div>
    </>
  );
}

const MARK: Record<Check["state"], string> = { ok: "✓", warn: "!", error: "✕", checking: "·", na: "–" };
const LABEL: Record<Check["state"], string> = {
  ok: "OK",
  warn: "Warning",
  error: "Error",
  checking: "Checking…",
  na: "—",
};
