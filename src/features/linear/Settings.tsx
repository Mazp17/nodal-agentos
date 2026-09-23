import { useEffect, useMemo, useState, type FormEvent } from "react";
import {
  configApi,
  linearApi,
  toLinearError,
  type AppConfig,
  type Effort,
  type FinishMode,
  type Issue,
  type RepoMapping,
  type Team,
  type Viewer,
} from "./api";
import { BrandMark } from "../../ui/BrandMark";
import { useToast } from "../../ui/Toasts";
import { pickRepoFolder, tildify, useHome } from "../runs/folders";
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

/** Lo editable de un mapeo, sin team/proyecto (que es la clave). */
type RepoSettings = Omit<RepoMapping, "teamId" | "projectId">;

interface Draft {
  teams: Record<string, RepoSettings>;
  projects: Record<string, RepoSettings>;
  /** Mapeos team+proyecto editados a mano en config.json: se preservan tal cual. */
  other: RepoMapping[];
  concurrency: number;
}

const settingsOf = ({ teamId: _t, projectId: _p, ...rest }: RepoMapping): RepoSettings => rest;

function toDraft(config: AppConfig): Draft {
  const d: Draft = { teams: {}, projects: {}, other: [], concurrency: config.concurrency };
  for (const m of config.repos) {
    if (m.teamId && !m.projectId) d.teams[m.teamId] = settingsOf(m);
    else if (m.projectId && !m.teamId) d.projects[m.projectId] = settingsOf(m);
    else d.other.push(m);
  }
  return d;
}

/** Sin campos vacíos: `undefined` = usar el default de Claude Code. */
function clean(s: RepoSettings): RepoSettings {
  const out: RepoSettings = { path: s.path.trim() };
  if (s.model) out.model = s.model;
  if (s.effort) out.effort = s.effort;
  if (s.permissionMode) out.permissionMode = s.permissionMode;
  if (s.finish) out.finish = s.finish;
  return out;
}

function fromDraft(d: Draft): AppConfig {
  const repos: RepoMapping[] = [
    ...Object.entries(d.teams)
      .filter(([, s]) => s.path.trim())
      .map(([teamId, s]) => ({ teamId, ...clean(s) })),
    ...Object.entries(d.projects)
      .filter(([, s]) => s.path.trim())
      .map(([projectId, s]) => ({ projectId, ...clean(s) })),
    ...d.other,
  ];
  return { repos, concurrency: d.concurrency };
}

function sameConfig(a: AppConfig, b: AppConfig): boolean {
  const key = (c: AppConfig) =>
    JSON.stringify([
      c.concurrency,
      c.repos
        .map((m) =>
          [m.teamId, m.projectId, m.path, m.model, m.effort, m.permissionMode, m.finish].map((x) => x ?? "").join("|"),
        )
        .sort(),
    ]);
  return key(a) === key(b);
}

/** Copia de `rec` con `key` puesto a `value` (o quitado si `undefined`). */
function withEntry(rec: Record<string, RepoSettings>, key: string, value: RepoSettings | undefined) {
  const next = { ...rec };
  if (value && value.path.trim()) next[key] = value;
  else delete next[key];
  return next;
}

const MODELS: { value: string; label: string }[] = [
  { value: "", label: "Claude default" },
  { value: "opus", label: "Opus" },
  { value: "sonnet", label: "Sonnet" },
  { value: "haiku", label: "Haiku" },
  { value: "fable", label: "Fable" },
];
const EFFORTS: { value: Effort | ""; label: string }[] = [
  { value: "", label: "Default" },
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "XHigh" },
  { value: "max", label: "Max" },
];
/** Valores de `claude --permission-mode` (`manual` es alias de `default`). */
const PERMISSION_MODES: { value: string; label: string; desc: string }[] = [
  { value: "", label: "Claude default", desc: "Whatever your Claude Code settings say." },
  { value: "default", label: "Ask for risky tools", desc: "The run pauses until you attach and answer each prompt." },
  { value: "acceptEdits", label: "Auto-accept edits", desc: "File edits run freely; shell commands still ask." },
  { value: "auto", label: "Auto", desc: "A classifier approves safe actions; risky ones still ask." },
  { value: "plan", label: "Plan only", desc: "Read-only: the agent plans but doesn't change anything." },
  { value: "dontAsk", label: "Don't ask", desc: "Anything not pre-approved is denied instead of prompting." },
  {
    value: "bypassPermissions",
    label: "Bypass permissions",
    desc: "Nothing asks. Only for sandboxed repos. Accept the bypass warning once in an interactive claude first.",
  },
];
const FINISH: { value: FinishMode; label: string }[] = [
  { value: "pr", label: "Open PR" },
  { value: "branch", label: "Branch only" },
];

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
  const home = useHome();

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
  const rowStatus = (cur: RepoSettings | undefined, prev: RepoSettings | undefined) => {
    const raw = cur?.path.trim() ?? "";
    const err = raw ? errorLines.find((l) => l.includes(`"${raw}"`)) : undefined;
    if (err) return { tone: "danger", text: err };
    if (raw !== (prev?.path ?? "")) return { tone: "warn", text: raw ? "Unsaved" : "Will be removed" };
    if (raw && JSON.stringify(clean(cur!)) !== JSON.stringify(clean(prev!))) return { tone: "warn", text: "Unsaved" };
    if (raw) return { tone: "ok", text: "Git repo · saved" };
    return { tone: "muted", text: "Not mapped" };
  };

  /** Mapeos editables (team o proyecto) con su setter, para Repos y Execution. */
  type Row = { key: string; kind: "Team" | "Project"; name: string; cur: RepoSettings | undefined; prev: RepoSettings | undefined; set: (s: RepoSettings | undefined) => void };
  const rows: Row[] = [
    ...p.teams.map((t): Row => ({
      key: `team-${t.id}`,
      kind: "Team",
      name: `${t.name} · ${t.key}`,
      cur: draft.teams[t.id],
      prev: saved.teams[t.id],
      set: (s) => setDraft((d) => ({ ...d, teams: withEntry(d.teams, t.id, s) })),
    })),
    ...projects.map(([id, name]): Row => ({
      key: `project-${id}`,
      kind: "Project",
      name,
      cur: draft.projects[id],
      prev: saved.projects[id],
      set: (s) => setDraft((d) => ({ ...d, projects: withEntry(d.projects, id, s) })),
    })),
  ];

  async function choose(r: Row) {
    try {
      const res = await pickRepoFolder(r.cur?.path);
      if (!res) return;
      if (!res.isRepo) {
        toast("Not a git repository", `${tildify(res.path, home)} — pick the root folder of a git checkout.`, "danger");
        return;
      }
      r.set({ ...(r.cur ?? {}), path: res.path });
      setSaveError(null);
    } catch (e) {
      toast("Couldn't open the folder picker", String(e), "danger");
    }
  }

  const repoRow = (r: Row) => {
    const st = rowStatus(r.cur, r.prev);
    const path = r.cur?.path ?? "";
    return (
      <div key={r.key} className="repo-cols repo-row">
        <span className="repo-scope">
          <span className="repo-kind">{r.kind}</span>
          <span className="ellipsis">{r.name}</span>
        </span>
        <button
          type="button"
          className="repo-pick"
          aria-label={`${path ? "Change" : "Choose"} the local repo for ${r.kind.toLowerCase()} ${r.name}`}
          title={path || undefined}
          onClick={() => void choose(r)}
        >
          <span className={`repo-pick-path ellipsis ${path ? "" : "empty"}`}>
            {path ? tildify(path, home) : r.kind === "Project" ? "Uses the team's repo" : "No repo"}
          </span>
          <span className="repo-pick-act">{path ? "Change…" : "Choose folder…"}</span>
        </button>
        <span className={`repo-status tone-${st.tone}`} title={st.text}>
          <span className="dot dot-sm" aria-hidden />
          <span className="ellipsis">{st.text}</span>
        </span>
        <span className="repo-acts">
          {path && (
            <button type="button" className="btn btn-sm btn-ghost repo-remove" onClick={() => r.set(undefined)}>
              Remove
            </button>
          )}
        </span>
      </div>
    );
  };

  const mapped = rows.filter((r) => r.cur?.path.trim());
  const setOpt = (r: Row, patch: Partial<RepoSettings>) => r.set({ ...r.cur!, ...patch });

  const execRepo = (r: Row) => {
    const s = r.cur!;
    const perm = PERMISSION_MODES.find((m) => m.value === (s.permissionMode ?? ""));
    const models = s.model && !MODELS.some((m) => m.value === s.model) ? [...MODELS, { value: s.model, label: s.model }] : MODELS;
    const st = rowStatus(r.cur, r.prev);
    return (
      <div key={r.key} className="exec-repo">
        <div className="exec-repo-head">
          <span className="repo-kind">{r.kind}</span>
          <span className="exec-repo-name ellipsis">{r.name}</span>
          <span className="exec-repo-path mono ellipsis" title={s.path}>
            {tildify(s.path, home)}
          </span>
          {st.tone !== "ok" && (
            <span className={`repo-status tone-${st.tone}`} title={st.text}>
              <span className="dot dot-sm" aria-hidden />
              <span className="ellipsis">{st.text}</span>
            </span>
          )}
        </div>
        <div className="exec-grid">
          <label className="exec-field">
            <span className="exec-field-label">Model</span>
            <select className="input exec-select" value={s.model ?? ""} onChange={(e) => setOpt(r, { model: e.target.value || undefined })}>
              {models.map((m) => (
                <option key={m.value} value={m.value}>
                  {m.label}
                </option>
              ))}
            </select>
          </label>
          <div className="exec-field">
            <span className="exec-field-label" id={`${r.key}-effort`}>
              Effort
            </span>
            <div className="seg" role="radiogroup" aria-labelledby={`${r.key}-effort`}>
              {EFFORTS.map((o) => (
                <button
                  key={o.value}
                  type="button"
                  role="radio"
                  aria-checked={(s.effort ?? "") === o.value}
                  className={`seg-opt ${(s.effort ?? "") === o.value ? "on" : ""}`}
                  onClick={() => setOpt(r, { effort: o.value || undefined })}
                >
                  {o.label}
                </button>
              ))}
            </div>
          </div>
          <label className="exec-field">
            <span className="exec-field-label">Permission mode</span>
            <select
              className="input exec-select"
              value={s.permissionMode ?? ""}
              onChange={(e) => setOpt(r, { permissionMode: e.target.value || undefined })}
            >
              {PERMISSION_MODES.map((m) => (
                <option key={m.value} value={m.value}>
                  {m.label}
                </option>
              ))}
              {perm === undefined && <option value={s.permissionMode}>{s.permissionMode}</option>}
            </select>
          </label>
          <div className="exec-field">
            <span className="exec-field-label" id={`${r.key}-finish`} title="Used by plan-task: open a PR or leave a branch">
              Finish
            </span>
            <div className="seg" role="radiogroup" aria-labelledby={`${r.key}-finish`}>
              {FINISH.map((o) => (
                <button
                  key={o.value}
                  type="button"
                  role="radio"
                  aria-checked={(s.finish ?? "pr") === o.value}
                  className={`seg-opt ${(s.finish ?? "pr") === o.value ? "on" : ""}`}
                  onClick={() => setOpt(r, { finish: o.value === "pr" ? undefined : o.value })}
                >
                  {o.label}
                </button>
              ))}
            </div>
          </div>
        </div>
        {perm && <p className="exec-desc exec-perm-desc">{perm.desc}</p>}
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
                {rows.map(repoRow)}
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
              <div className="settings-title-row">
                <h3 className="settings-subtitle">Per repo</h3>
                <p className="settings-desc">
                  Passed to <span className="mono">claude --bg</span> for runs in that repo. “Finish” is used by
                  plan-task: open a PR or leave a branch.
                </p>
              </div>
              {mapped.length === 0 ? (
                <div className="panel repo-empty">Map a repo in Settings → Repos first.</div>
              ) : (
                <div className="panel">{mapped.map(execRepo)}</div>
              )}
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
