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

/** Form para pegar/reemplazar la key. Se usa en Ajustes y en el onboarding. */
export function ApiKeyForm({
  configured,
  onSaved,
  onCleared,
}: {
  configured: boolean;
  onSaved: (viewer: Viewer) => void;
  onCleared?: () => void;
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
      const viewer = await linearApi.setApiKey(key);
      setKey("");
      onSaved(viewer);
    } catch (err) {
      setError(toLinearError(err).message);
    } finally {
      setBusy(false);
    }
  }

  async function clear() {
    setBusy(true);
    setError(null);
    try {
      await linearApi.clearApiKey();
      onCleared?.();
    } catch (err) {
      setError(toLinearError(err).message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="key-form" onSubmit={submit}>
      <div className="row">
        <input
          type="password"
          autoComplete="off"
          spellCheck={false}
          placeholder={configured ? "Pegá una key nueva para reemplazarla" : "lin_api_…"}
          value={key}
          onChange={(e) => setKey(e.target.value)}
          disabled={busy}
          aria-label="API key de Linear"
        />
        <button className="btn primary" type="submit" disabled={busy || !key.trim()}>
          {busy ? "Probando…" : "Guardar y probar"}
        </button>
        {configured && onCleared && (
          <button className="btn danger" type="button" onClick={clear} disabled={busy}>
            Borrar key
          </button>
        )}
      </div>
      <p className="hint">
        Linear → Settings → Security &amp; access → Personal API keys. Se guarda en el llavero
        de macOS; con permiso de lectura alcanza.
      </p>
      {error && <p className="form-error">{error}</p>}
    </form>
  );
}

export function Onboarding({ onSaved }: { onSaved: (viewer: Viewer) => void }) {
  return (
    <div className="state-panel onboarding">
      <h2>Conectá Linear</h2>
      <p>Pegá una API key personal para ver tus issues en el board.</p>
      <ApiKeyForm configured={false} onSaved={onSaved} />
    </div>
  );
}

interface Draft {
  teams: Record<string, string>;
  projects: Record<string, string>;
  /** Mapeos team+proyecto editados a mano en config.json: se preservan tal cual. */
  other: RepoMapping[];
  concurrency: string;
}

function toDraft(config: AppConfig): Draft {
  const d: Draft = { teams: {}, projects: {}, other: [], concurrency: String(config.concurrency) };
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
  return { repos, concurrency: Number(d.concurrency) };
}

export function SettingsView({
  teams,
  issues,
  config,
  configLoaded,
  keyConfigured,
  onConfigSaved,
  onKeySaved,
  onKeyCleared,
}: {
  teams: Team[];
  issues: Issue[];
  config: AppConfig;
  configLoaded: boolean;
  keyConfigured: boolean;
  onConfigSaved: (config: AppConfig) => void;
  /** Key reemplazada (ya validada y guardada). */
  onKeySaved: () => void;
  onKeyCleared: () => void;
}) {
  const [draft, setDraft] = useState<Draft>(() => toDraft(config));
  const [saving, setSaving] = useState(false);
  const [result, setResult] = useState<{ ok: boolean; text: string } | null>(null);
  const [viewer, setViewer] = useState<Viewer | null>(null);
  const [viewerError, setViewerError] = useState<string | null>(null);

  useEffect(() => setDraft(toDraft(config)), [config]);

  useEffect(() => {
    if (!keyConfigured) {
      setViewer(null);
      return;
    }
    linearApi
      .viewer()
      .then((v) => {
        setViewer(v);
        setViewerError(null);
      })
      .catch((e) => setViewerError(toLinearError(e).message));
  }, [keyConfigured]);

  // Proyectos vistos en el board (Linear no tiene un listado barato por team) más
  // los que ya tienen mapeo aunque hoy no tengan issues visibles.
  const projects = useMemo(() => {
    const byId = new Map<string, string>();
    for (const i of issues) if (i.project) byId.set(i.project.id, i.project.name);
    for (const id of Object.keys(draft.projects)) if (!byId.has(id)) byId.set(id, id);
    return [...byId].sort((a, b) => a[1].localeCompare(b[1]));
  }, [issues, draft.projects]);

  async function save(e: FormEvent) {
    e.preventDefault();
    const concurrency = Number(draft.concurrency);
    if (!Number.isInteger(concurrency) || concurrency < 1 || concurrency > 16) {
      setResult({ ok: false, text: "Runs en paralelo: un entero entre 1 y 16." });
      return;
    }
    setSaving(true);
    setResult(null);
    try {
      const saved = await configApi.save(fromDraft(draft));
      onConfigSaved(saved);
      setResult({ ok: true, text: "Guardado." });
    } catch (err) {
      setResult({ ok: false, text: String(err) });
    } finally {
      setSaving(false);
    }
  }

  return (
    <div className="settings">
      <section className="settings-section">
        <h2>API key de Linear</h2>
        <p className="status-line">
          {!keyConfigured
            ? "Sin key configurada."
            : viewer
              ? `Conectado como ${viewer.name} (${viewer.email})`
              : viewerError
                ? <span className="form-error">{viewerError}</span>
                : "Verificando…"}
        </p>
        <ApiKeyForm
          configured={keyConfigured}
          onSaved={(v) => {
            setViewer(v);
            setViewerError(null);
            onKeySaved();
          }}
          onCleared={onKeyCleared}
        />
      </section>

      <form className="settings-section" onSubmit={save}>
        <h2>Repos por team</h2>
        <p className="hint">Ruta absoluta a la raíz de un repo git. Vacío = sin repo.</p>
        {teams.length === 0 && <p className="empty">No hay teams cargados.</p>}
        {teams.map((t) => (
          <label key={t.id} className="map-row">
            <span className="map-label">
              <span className="chip">{t.key}</span> {t.name}
            </span>
            <input
              type="text"
              spellCheck={false}
              placeholder="/Users/…/repo"
              value={draft.teams[t.id] ?? ""}
              onChange={(e) =>
                setDraft({ ...draft, teams: { ...draft.teams, [t.id]: e.target.value } })
              }
            />
          </label>
        ))}

        <h2>Repos por proyecto <span className="muted">(opcional, gana sobre el del team)</span></h2>
        {projects.length === 0 && <p className="empty">No hay proyectos en el board.</p>}
        {projects.map(([id, name]) => (
          <label key={id} className="map-row">
            <span className="map-label">{name}</span>
            <input
              type="text"
              spellCheck={false}
              placeholder="Usa el repo del team"
              value={draft.projects[id] ?? ""}
              onChange={(e) =>
                setDraft({ ...draft, projects: { ...draft.projects, [id]: e.target.value } })
              }
            />
          </label>
        ))}
        {draft.other.length > 0 && (
          <>
            <h2>Mapeos team + proyecto <span className="muted">(editados en config.json)</span></h2>
            {draft.other.map((m, i) => (
              <div key={`${m.teamId}-${m.projectId}-${i}`} className="map-row">
                <span className="map-label">
                  {teams.find((t) => t.id === m.teamId)?.key ?? m.teamId} ·{" "}
                  {projects.find(([id]) => id === m.projectId)?.[1] ?? m.projectId}
                </span>
                <span className="other-row">
                  <code>{m.path}</code>
                  <button
                    type="button"
                    className="btn danger"
                    onClick={() =>
                      setDraft({ ...draft, other: draft.other.filter((_, j) => j !== i) })
                    }
                  >
                    Quitar
                  </button>
                </span>
              </div>
            ))}
          </>
        )}

        <h2>Ejecución</h2>
        <label className="map-row">
          <span className="map-label">Runs en paralelo</span>
          <input
            type="number"
            min={1}
            max={16}
            className="narrow"
            value={draft.concurrency}
            onChange={(e) => setDraft({ ...draft, concurrency: e.target.value })}
          />
        </label>

        <div className="actions">
          <button className="btn primary" type="submit" disabled={saving || !configLoaded}>
            {saving ? "Guardando…" : "Guardar"}
          </button>
          {result && <span className={result.ok ? "ok-text" : "form-error pre"}>{result.text}</span>}
        </div>
      </form>
    </div>
  );
}
