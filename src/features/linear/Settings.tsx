import { useEffect, useMemo, useState, type FormEvent } from "react";
import {
  configApi,
  linearApi,
  toLinearError,
  type AppConfig,
  type Issue,
  type RepoMapping,
  type Team,
  type Viewer,
} from "./api";
import { BrandMark } from "../../ui/BrandMark";
import { useToast } from "../../ui/Toasts";
import "./settings.css";

export type SettingsSection = "linear" | "repos" | "exec" | "diag";

const SECTIONS: { id: SettingsSection; label: string }[] = [
  { id: "linear", label: "Linear account" },
  { id: "repos", label: "Repos" },
  { id: "exec", label: "Execution" },
  { id: "diag", label: "Diagnostics" },
];

const MAX_CONCURRENCY = 16;

/** Form para pegar/reemplazar la key. Se usa en Ajustes y en el onboarding. */
export function ApiKeyForm({
  onSaved,
  onCancel,
  autoFocus,
}: {
  onSaved: (viewer: Viewer) => void;
  onCancel?: () => void;
  autoFocus?: boolean;
}) {
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!key.trim()) return;
    setBusy(true);
    setError(null);
    try {
      const viewer = await linearApi.setApiKey(key.trim());
      setKey("");
      onSaved(viewer);
    } catch (err) {
      setError(toLinearError(err).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="key-form" onSubmit={submit}>
      <div className="key-form-row">
        <input
          className={`input input-mono key-input ${error ? "input-invalid" : ""}`}
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder="lin_api_…"
          value={key}
          onChange={(e) => {
            setKey(e.target.value);
            setError(null);
          }}
          disabled={busy}
          aria-label="Linear personal API key"
          aria-invalid={error !== null}
          aria-describedby="key-hint"
          autoFocus={autoFocus}
        />
        <button className="btn btn-primary btn-lg" type="submit" disabled={busy || !key.trim()}>
          {busy ? "Testing…" : "Save and test"}
        </button>
        {onCancel && (
          <button className="btn btn-lg" type="button" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
        )}
      </div>
      <span id="key-hint" className={error ? "key-msg key-msg-error" : "key-msg"} role={error ? "alert" : undefined}>
        {error ??
          (busy
            ? "Testing against api.linear.app…"
            : "Linear → Settings → Security & access → Personal API keys. Stored in the macOS Keychain, never on disk.")}
      </span>
    </form>
  );
}

export function Onboarding({ onSaved }: { onSaved: (viewer: Viewer) => void }) {
  return (
    <div className="onboarding">
      <div className="onboarding-card">
        <BrandMark size={40} />
        <h1 className="onboarding-title">Connect Linear</h1>
        <p className="onboarding-text">
          Agent Desk pulls your issues from Linear and launches Claude Code workflows on them. Paste a personal API key
          to get started — read access is enough.
        </p>
        <ApiKeyForm onSaved={onSaved} autoFocus />
      </div>
    </div>
  );
}

interface Draft {
  teams: Record<string, string>;
  projects: Record<string, string>;
  /** Mapeos team+proyecto editados a mano en config.json: se preservan tal cual. */
  other: RepoMapping[];
  concurrency: number;
}

function toDraft(config: AppConfig): Draft {
  const d: Draft = { teams: {}, projects: {}, other: [], concurrency: config.concurrency };
  for (const m of config.repos) {
    if (m.teamId && !m.projectId) d.teams[m.teamId] = m.path;
    else if (m.projectId && !m.teamId) d.projects[m.projectId] = m.path;
    else d.other.push(m);
  }
  return d;
}

function fromDraft(d: Draft): AppConfig {
  const repos: RepoMapping[] = [
    ...Object.entries(d.teams)
      .filter(([, p]) => p.trim())
      .map(([teamId, path]) => ({ teamId, path: path.trim() })),
    ...Object.entries(d.projects)
      .filter(([, p]) => p.trim())
      .map(([projectId, path]) => ({ projectId, path: path.trim() })),
    ...d.other,
  ];
  return { repos, concurrency: d.concurrency };
}

function sameConfig(a: AppConfig, b: AppConfig): boolean {
  const key = (c: AppConfig) =>
    JSON.stringify([
      c.concurrency,
      c.repos.map((m) => `${m.teamId ?? ""}|${m.projectId ?? ""}|${m.path}`).sort(),
    ]);
  return key(a) === key(b);
}

export interface Diagnostics {
  cli: { ok: boolean; text: string } | null;
  workflows: number | null;
}

interface Props {
  section: SettingsSection;
  onSection: (s: SettingsSection) => void;
  teams: Team[];
  issues: Issue[];
  config: AppConfig;
  configLoaded: boolean;
  configError: string | null;
  viewer: Viewer | null;
  viewerError: string | null;
  diagnostics: Diagnostics;
  onRecheck: () => Promise<void>;
  onConfigSaved: (config: AppConfig) => void;
  /** Key reemplazada (ya validada y guardada). */
  onKeySaved: (viewer: Viewer) => void;
  onKeyCleared: () => void;
}

export function SettingsView(p: Props) {
  const toast = useToast();
  const [draft, setDraft] = useState<Draft>(() => toDraft(p.config));
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [replacing, setReplacing] = useState(false);
  const [clearing, setClearing] = useState(false);
  const [checking, setChecking] = useState(false);

  useEffect(() => setDraft(toDraft(p.config)), [p.config]);

  // Proyectos vistos en el board (Linear no tiene un listado barato por team) más
  // los que ya tienen mapeo aunque hoy no tengan issues visibles.
  const projects = useMemo(() => {
    const byId = new Map<string, string>();
    for (const i of p.issues) if (i.project) byId.set(i.project.id, i.project.name);
    for (const id of Object.keys(draft.projects)) if (!byId.has(id)) byId.set(id, id);
    return [...byId].sort((a, b) => a[1].localeCompare(b[1]));
  }, [p.issues, draft.projects]);

  const saved = useMemo(() => toDraft(p.config), [p.config]);
  const dirty = !sameConfig(fromDraft(draft), fromDraft(saved));
  const errorLines = saveError ? saveError.split("\n").filter(Boolean) : [];

  async function save() {
    setSaving(true);
    setSaveError(null);
    try {
      const next = await configApi.save(fromDraft(draft));
      p.onConfigSaved(next);
      toast("Settings saved", undefined, "ok");
    } catch (err) {
      setSaveError(String(err));
    } finally {
      setSaving(false);
    }
  }

  async function clearKey() {
    if (!window.confirm("Delete the Linear key from the Keychain? The board stops syncing until you add one.")) return;
    setClearing(true);
    try {
      await linearApi.clearApiKey();
      toast("Linear key deleted", "Removed from the Keychain.", "danger");
      p.onKeyCleared();
    } catch (err) {
      toast("Couldn't delete the key", toLinearError(err).message, "danger");
    } finally {
      setClearing(false);
    }
  }

  const saveBar = (
    <div className="save-bar">
      {errorLines.length > 0 && (
        <ul className="save-errors" role="alert">
          {errorLines.map((l, i) => (
            <li key={i}>{l}</li>
          ))}
        </ul>
      )}
      <div className="save-bar-row">
        <span className="save-state">
          {!p.configLoaded
            ? p.configError
              ? "The config file couldn't be read; saving is disabled so it isn't overwritten."
              : "Loading config…"
            : dirty
              ? "Unsaved changes"
              : "All changes saved"}
        </span>
        {dirty && (
          <button type="button" className="btn" onClick={() => setDraft(saved)} disabled={saving}>
            Discard
          </button>
        )}
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => void save()}
          disabled={saving || !dirty || !p.configLoaded}
        >
          {saving ? "Saving…" : "Save changes"}
        </button>
      </div>
    </div>
  );

  /** Estado por fila: error del último guardado, sin guardar, guardado o sin mapear. */
  const rowStatus = (path: string, savedPath: string | undefined) => {
    const raw = path.trim();
    const err = raw ? errorLines.find((l) => l.includes(`«${raw}»`)) : undefined;
    if (err) return { tone: "danger", text: err };
    if (raw !== (savedPath ?? "")) return { tone: "warn", text: raw ? "Unsaved" : "Will be removed" };
    if (raw) return { tone: "ok", text: "Git repo · saved" };
    return { tone: "muted", text: "Not mapped" };
  };

  const repoRow = (kind: string, id: string, name: string, path: string, savedPath: string | undefined, onChange: (v: string) => void) => {
    const st = rowStatus(path, savedPath);
    return (
      <div key={`${kind}-${id}`} className="repo-cols repo-row">
        <span className="repo-scope">
          <span className="repo-kind">{kind}</span>
          <span className="ellipsis">{name}</span>
        </span>
        <input
          className="input input-mono repo-path"
          spellCheck={false}
          placeholder={kind === "Project" ? "Uses the team's repo" : "/Users/…/repo"}
          aria-label={`Local path for ${kind.toLowerCase()} ${name}`}
          value={path}
          onChange={(e) => onChange(e.target.value)}
        />
        <span className={`repo-status tone-${st.tone}`} title={st.text}>
          <span className="dot dot-sm" aria-hidden />
          <span className="ellipsis">{st.text}</span>
        </span>
        <span className="repo-acts">
          {path && (
            <button type="button" className="btn btn-sm btn-ghost repo-remove" onClick={() => onChange("")}>
              Remove
            </button>
          )}
        </span>
      </div>
    );
  };

  const connected = p.viewer !== null;
  const linDot = connected ? "ok" : p.viewerError ? "danger" : "muted";

  const diag: { name: string; value: string; tone: "ok" | "warn" | "danger" | "muted"; status: string }[] = [
    p.diagnostics.cli === null
      ? { name: "Claude Code CLI", value: "Looking for claude…", tone: "muted", status: "Checking…" }
      : p.diagnostics.cli.ok
        ? { name: "Claude Code CLI", value: p.diagnostics.cli.text, tone: "ok", status: "OK" }
        : { name: "Claude Code CLI", value: p.diagnostics.cli.text, tone: "danger", status: "Error" },
    connected
      ? { name: "Linear API", value: `${p.viewer!.name} · ${p.viewer!.email}`, tone: "ok", status: "OK" }
      : p.viewerError
        ? { name: "Linear API", value: p.viewerError, tone: "danger", status: "Error" }
        : { name: "Linear API", value: "Checking…", tone: "muted", status: "Checking…" },
    {
      name: "Repos",
      value: `${p.config.repos.length} mapping${p.config.repos.length === 1 ? "" : "s"} · validated on save`,
      tone: p.config.repos.length ? "ok" : "warn",
      status: p.config.repos.length ? "OK" : "Warning",
    },
    p.diagnostics.workflows === null
      ? { name: "Workflows", value: "~/.claude/workflows", tone: "muted", status: "—" }
      : {
          name: "Workflows",
          value: `${p.diagnostics.workflows} in ~/.claude/workflows`,
          tone: p.diagnostics.workflows ? "ok" : "warn",
          status: p.diagnostics.workflows ? "OK" : "Warning",
        },
  ];
  const MARK = { ok: "✓", warn: "!", danger: "✕", muted: "·" } as const;

  return (
    <div className="settings">
      <nav className="settings-nav" aria-label="Settings sections">
        {SECTIONS.map((s) => (
          <button
            key={s.id}
            type="button"
            className={`settings-nav-item ${p.section === s.id ? "on" : ""}`}
            aria-current={p.section === s.id ? "page" : undefined}
            onClick={() => p.onSection(s.id)}
          >
            {s.label}
          </button>
        ))}
      </nav>
      <div className="settings-body">
        <div className="settings-inner">
          {p.section === "linear" && (
            <>
              <h2 className="settings-title">Linear account</h2>
              <div className="panel settings-card">
                <div className="lin-status">
                  <span className={`dot dot-lg tone-${linDot}`} aria-hidden />
                  <span className="lin-title">
                    {connected ? `Connected as ${p.viewer!.name}` : p.viewerError ? "Can't verify the key" : "Checking key…"}
                  </span>
                  <span className="lin-sub">{connected ? p.viewer!.email : p.viewerError}</span>
                </div>
                <div className="key-stored">
                  <span className="key-stored-mask">lin_api_••••••••••••••••••••</span>
                  <span className="key-stored-note">Stored in macOS Keychain</span>
                </div>
                {replacing ? (
                  <ApiKeyForm
                    autoFocus
                    onCancel={() => setReplacing(false)}
                    onSaved={(v) => {
                      setReplacing(false);
                      toast("Linear connected", `Signed in as ${v.name}`, "ok");
                      p.onKeySaved(v);
                    }}
                  />
                ) : (
                  <div className="settings-actions">
                    <button type="button" className="btn" onClick={() => setReplacing(true)}>
                      Replace key
                    </button>
                    <button type="button" className="btn btn-danger" onClick={() => void clearKey()} disabled={clearing}>
                      {clearing ? "Deleting…" : "Delete key"}
                    </button>
                  </div>
                )}
              </div>
            </>
          )}

          {p.section === "repos" && (
            <>
              <div className="settings-title-row">
                <h2 className="settings-title">Repos</h2>
                <p className="settings-desc">
                  Map a Linear team or project to a local git checkout. Project mappings win over team mappings.
                </p>
              </div>
              <div className="panel repo-table">
                <div className="table-head repo-cols">
                  <span>Linear scope</span>
                  <span>Local path</span>
                  <span>Validation</span>
                  <span />
                </div>
                {p.teams.length === 0 && <div className="repo-empty">No teams loaded from Linear yet.</div>}
                {p.teams.map((t) =>
                  repoRow("Team", t.id, `${t.name} · ${t.key}`, draft.teams[t.id] ?? "", saved.teams[t.id], (v) =>
                    setDraft({ ...draft, teams: { ...draft.teams, [t.id]: v } }),
                  ),
                )}
                {projects.map(([id, name]) =>
                  repoRow("Project", id, name, draft.projects[id] ?? "", saved.projects[id], (v) =>
                    setDraft({ ...draft, projects: { ...draft.projects, [id]: v } }),
                  ),
                )}
                {draft.other.map((m, i) => (
                  <div key={`${m.teamId}-${m.projectId}-${i}`} className="repo-cols repo-row">
                    <span className="repo-scope">
                      <span className="repo-kind">Team + project · edited in config.json</span>
                      <span className="ellipsis">
                        {p.teams.find((t) => t.id === m.teamId)?.key ?? m.teamId} ·{" "}
                        {projects.find(([id]) => id === m.projectId)?.[1] ?? m.projectId}
                      </span>
                    </span>
                    <span className="repo-path-ro mono ellipsis" title={m.path}>
                      {m.path}
                    </span>
                    <span className="repo-status tone-ok">
                      <span className="dot dot-sm" aria-hidden />
                      Saved
                    </span>
                    <span className="repo-acts">
                      <button
                        type="button"
                        className="btn btn-sm btn-ghost repo-remove"
                        onClick={() => setDraft({ ...draft, other: draft.other.filter((_, j) => j !== i) })}
                      >
                        Remove
                      </button>
                    </span>
                  </div>
                ))}
              </div>
              {saveBar}
            </>
          )}

          {p.section === "exec" && (
            <>
              <h2 className="settings-title">Execution</h2>
              <div className="panel">
                <div className="exec-row">
                  <div className="exec-label">
                    <span id="par-label">Parallel runs</span>
                    <span className="exec-desc">Extra launches wait in the queue.</span>
                  </div>
                  <div className="stepper" role="group" aria-labelledby="par-label">
                    <button
                      type="button"
                      aria-label="Fewer parallel runs"
                      disabled={draft.concurrency <= 1}
                      onClick={() => setDraft({ ...draft, concurrency: Math.max(1, draft.concurrency - 1) })}
                    >
                      −
                    </button>
                    <span className="num" aria-live="polite">
                      {draft.concurrency}
                    </span>
                    <button
                      type="button"
                      aria-label="More parallel runs"
                      disabled={draft.concurrency >= MAX_CONCURRENCY}
                      onClick={() =>
                        setDraft({ ...draft, concurrency: Math.min(MAX_CONCURRENCY, draft.concurrency + 1) })
                      }
                    >
                      +
                    </button>
                  </div>
                </div>
              </div>
              {saveBar}
            </>
          )}

          {p.section === "diag" && (
            <>
              <div className="settings-title-row settings-title-row-inline">
                <h2 className="settings-title">Diagnostics</h2>
                <button
                  type="button"
                  className="btn"
                  disabled={checking}
                  onClick={async () => {
                    setChecking(true);
                    try {
                      await p.onRecheck();
                    } finally {
                      setChecking(false);
                    }
                  }}
                >
                  {checking ? "Checking…" : "Run again"}
                </button>
              </div>
              <div className="panel">
                {diag.map((d) => (
                  <div key={d.name} className={`diag-row tone-${d.tone}`}>
                    <span className="diag-mark" aria-hidden>
                      {checking ? "·" : MARK[d.tone]}
                    </span>
                    <span>{d.name}</span>
                    <span className="diag-value ellipsis" title={d.value}>
                      {d.value}
                    </span>
                    <span className="diag-status">{checking ? "Checking…" : d.status}</span>
                  </div>
                ))}
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
