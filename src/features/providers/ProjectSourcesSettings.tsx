import { useState } from "react";
import { useRepos } from "../../domain/hooks/store";
import { createSourceLink, deleteSourceLink, syncNow, updateSourceLink, type SourceLinkPatch } from "../../domain/api";
import type { ExternalState, Repo, RepoRule, SourceLink } from "../../domain/types";
import {
  errorText,
  invalidateProviders,
  useProviderScopes,
  useProviderStatus,
  useRuleProjects,
  useNow,
  useSourceLinks,
  useSourceStates,
} from "../../domain/hooks/providers";
import { useToast } from "../../ui/Toasts";
import { Field, InlineConfirm, ProviderMark, Segmented, Switch } from "./parts";
import { ScopePicker } from "./ScopePicker";
import { StateMapEditor } from "./StateMapEditor";
import { formatAgo, plural, providerName } from "./meta";
import { newProjectRule, useRuleBackfill } from "./ruleBackfill";

export interface ProjectSourcesSettingsProps {
  projectId: string;
  /** "Fix key" / "Connect Linear": goes to Settings → Integrations. */
  onOpenIntegrations?: () => void;
}

/** Project settings → Sources. */
export function ProjectSourcesSettings({ projectId, onOpenIntegrations }: ProjectSourcesSettingsProps) {
  const links = useSourceLinks(projectId);
  const repos = useRepos(projectId);
  const [connecting, setConnecting] = useState(false);
  /** Newly connected link: opens its mapping. */
  const [justConnected, setJustConnected] = useState<string | null>(null);

  return (
    <section className="pv-page" aria-labelledby="pv-src-title">
      <header className="pv-page-head pv-page-head-row">
        <div>
          <h2 id="pv-src-title" className="pv-page-title">
            Sources
          </h2>
          <p className="pv-page-sub">
            Task managers that feed this project. Imported tasks stay linked: Nodal pushes run state back and pulls
            title, description and state changes.
          </p>
        </div>
        <button type="button" className="btn" aria-expanded={connecting} onClick={() => setConnecting((v) => !v)}>
          Connect source
        </button>
      </header>

      {connecting && (
        <ConnectSource
          projectId={projectId}
          repos={repos.data ?? []}
          existing={links.data ?? []}
          onOpenIntegrations={onOpenIntegrations}
          onCancel={() => setConnecting(false)}
          onConnected={(l) => {
            setConnecting(false);
            setJustConnected(l.id);
          }}
        />
      )}

      {links.error && (
        <div className="pv-alert pv-alert-danger" role="alert">
          <span className="pv-alert-text">Could not load sources. {links.error}</span>
          <button type="button" className="btn btn-sm" onClick={links.reload}>
            Retry
          </button>
        </div>
      )}
      {!links.data && links.loading && <div className="pv-empty">Loading sources…</div>}

      {links.data?.map((l) => (
        <SourceCard
          key={l.id}
          link={l}
          repos={repos.data ?? []}
          openMapInitially={l.id === justConnected}
          onOpenIntegrations={onOpenIntegrations}
        />
      ))}

      {links.data?.length === 0 && !connecting && (
        <div className="pv-empty pv-empty-dashed">
          No sources. Tasks in this project are created in Nodal. Connect Linear to import issues and keep them in sync.
        </div>
      )}
    </section>
  );
}

// ---------- Connect ----------

function ConnectSource({
  projectId,
  repos,
  existing,
  onCancel,
  onConnected,
  onOpenIntegrations,
}: {
  projectId: string;
  repos: Repo[];
  existing: SourceLink[];
  onCancel: () => void;
  onConnected: (l: SourceLink) => void;
  onOpenIntegrations?: () => void;
}) {
  const toast = useToast();
  const st = useProviderStatus("linear");
  const connected = st.connection === "connected";
  const scopes = useProviderScopes("linear", connected);
  const [scopeId, setScopeId] = useState<string | null>(null);
  const [repoId, setRepoId] = useState<string>(repos[0]?.id ?? "");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const taken = new Set(existing.filter((l) => l.provider === "linear").map((l) => l.scope.id));
  const available = scopes.data?.filter((s) => !taken.has(s.id)) ?? null;
  const scope = available?.find((s) => s.id === scopeId) ?? null;
  const defaultRepo = repoId || repos[0]?.id || "";

  const connect = async () => {
    if (!scope) return;
    setBusy(true);
    setError(null);
    try {
      const link = await createSourceLink({
        projectId,
        provider: "linear",
        scope,
        defaultRepoId: defaultRepo || null,
      });
      invalidateProviders("links");
      toast("Source connected", `Linear · ${scope.name}. Review the status mapping to start syncing state.`, "ok");
      onConnected(link);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pv-card">
      <div className="pv-card-body">
        <div className="pv-card-row">
          <ProviderMark />
          <span className="pv-card-name">Linear</span>
          <span className="pv-hint">Asana and Azure DevOps coming soon</span>
        </div>
        {st.connection === "loading" && <span className="pv-hint">Checking Linear…</span>}
        {!connected && st.connection !== "loading" && (
          <div className="pv-alert">
            <span className="pv-alert-text">
              {st.status?.hasKey
                ? `The Linear key is not working${st.error ? `: ${st.error}` : "."}`
                : "Connect your Linear account in Settings → Integrations first."}
            </span>
            {onOpenIntegrations && (
              <button type="button" className="btn btn-sm" onClick={onOpenIntegrations}>
                Open Integrations
              </button>
            )}
          </div>
        )}
        {connected && (
          <>
            <Field label="Team or project">
              <ScopePicker
                scopes={available}
                loading={scopes.loading}
                error={scopes.error}
                value={scopeId}
                onChange={setScopeId}
              />
              {available && available.length === 0 && (
                <span className="pv-hint">Every team and project is already connected here.</span>
              )}
            </Field>
            <Field label="Default repo for imports">
              <RepoChoice repos={repos} value={defaultRepo || null} onChange={(v) => setRepoId(v ?? "")} label="Default repo" />
            </Field>
          </>
        )}
        {error && (
          <div className="pv-alert pv-alert-danger" role="alert">
            {error}
          </div>
        )}
        <div className="pv-actions pv-actions-end">
          <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          {connected && (
            <button type="button" className="btn btn-primary btn-sm" disabled={!scope || busy} onClick={() => void connect()}>
              {busy ? "Connecting…" : "Connect"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// ---------- Source card ----------

function SourceCard({
  link,
  repos,
  openMapInitially,
  onOpenIntegrations,
}: {
  link: SourceLink;
  repos: Repo[];
  openMapInitially: boolean;
  onOpenIntegrations?: () => void;
}) {
  const toast = useToast();
  const prov = providerName(link.provider);
  const st = useProviderStatus(link.provider);
  const keyOk = st.connection === "connected";
  const [mapOpen, setMapOpen] = useState(openMapInitially);
  // `source_states` hits the network: only with the mapping editor open.
  const states = useSourceStates(keyOk && mapOpen ? link.id : null);
  const [confirmDisconnect, setConfirmDisconnect] = useState(false);
  const [busy, setBusy] = useState<"patch" | "disconnect" | "sync" | null>(null);
  const now = useNow();

  const pending = link.stateMap.confirmedAt === null;
  const report = states.data;
  // State additions/removals detected by the last sync (cleared when the mapping is saved).
  const changes = link.pendingStateChanges;
  const added = changes?.added.length ?? 0;
  const removed = changes?.removed.length ?? 0;
  const drift = added + removed;

  const patch = async (p: SourceLinkPatch) => {
    setBusy("patch");
    try {
      await updateSourceLink(link.id, p);
      invalidateProviders("links");
    } catch (err) {
      toast("Could not update the source", errorText(err), "danger");
    } finally {
      setBusy(null);
    }
  };

  const disconnect = async () => {
    setBusy("disconnect");
    try {
      await deleteSourceLink(link.id);
      invalidateProviders("links");
      toast("Source disconnected", `${prov} · ${link.scope.name}. Imported tasks stay as local tasks.`, "warn");
    } catch (err) {
      toast("Could not disconnect", errorText(err), "danger");
      setBusy(null);
    }
  };

  const sync = async () => {
    setBusy("sync");
    try {
      const r = await syncNow(link.id);
      invalidateProviders("links");
      if (mapOpen) states.reload();
      const parts = [
        r.pulled ? `${r.pulled} pulled` : null,
        r.pushed ? `${r.pushed} pushed` : null,
        r.imported ? `${r.imported} imported` : null,
      ].filter(Boolean);
      const extra = [...r.errors, ...r.notices].join("\n");
      toast(
        r.errors.length ? "Sync finished with errors" : "Synced",
        [parts.join(" · ") || "Nothing changed.", extra].filter(Boolean).join("\n"),
        r.errors.length ? "danger" : "ok",
      );
    } catch (err) {
      toast("Sync failed", errorText(err), "danger");
    } finally {
      setBusy(null);
    }
  };

  const hasRepoRoute = !!link.defaultRepoId || link.repoRules.length > 0;

  return (
    <article className="pv-card" aria-label={`${prov} · ${link.scope.name}`}>
      <div className="pv-card-body">
        <div className="pv-card-row">
          <ProviderMark />
          <span className="pv-card-name">{prov}</span>
          <span className="pv-scope">
            {link.scope.name}
            <span className="pv-kind">{link.scope.kind}</span>
          </span>
          <SourceStatus
            keyOk={keyOk}
            keyLoading={st.connection === "loading"}
            pending={pending}
            drift={drift}
            syncedAt={link.lastSyncedAt}
            syncFailed={!!link.lastSyncError}
            now={now}
          />
          <span className="pv-spacer" />
          <button type="button" className="btn btn-sm" disabled={!keyOk || busy !== null} onClick={() => void sync()}>
            {busy === "sync" ? "Syncing…" : "Sync now"}
          </button>
          <button
            type="button"
            className="btn btn-ghost btn-sm"
            disabled={busy !== null}
            onClick={() => setConfirmDisconnect(true)}
          >
            Disconnect
          </button>
        </div>

        {confirmDisconnect && (
          <InlineConfirm
            text={`Disconnect ${prov} · ${link.scope.name}? Imported tasks stay in this project as local tasks; nothing changes in ${prov}.`}
            confirmLabel="Disconnect"
            busy={busy === "disconnect"}
            onConfirm={() => void disconnect()}
            onCancel={() => setConfirmDisconnect(false)}
          />
        )}

        {!keyOk && st.connection !== "loading" && (
          <div className="pv-alert pv-alert-danger" role="alert">
            <span className="pv-alert-text">
              {st.status?.hasKey
                ? `${prov} is not reachable with the saved key${st.error ? `: ${st.error}` : "."} Tasks keep their last synced state.`
                : `${prov} is disconnected. Tasks keep their last synced state.`}
            </span>
            {onOpenIntegrations && (
              <button type="button" className="btn btn-sm" onClick={onOpenIntegrations}>
                Fix key
              </button>
            )}
          </div>
        )}

        {keyOk && link.lastSyncError && (
          <div className="pv-alert pv-alert-danger" role="alert">
            <span className="pv-alert-text">
              Last sync failed{link.lastSyncedAt ? ` (last success ${formatAgo(link.lastSyncedAt, now)})` : ""}: {link.lastSyncError}
            </span>
            <button type="button" className="btn btn-sm" disabled={busy !== null} onClick={() => void sync()}>
              Retry
            </button>
          </div>
        )}

        {!mapOpen && drift > 0 && changes && (
          <div className="pv-alert pv-alert-warn" role="status">
            <span className="pv-alert-text">
              {[
                added ? `${plural(added, "new state")} in ${prov}${stateNames(changes.added)}` : null,
                removed ? `${plural(removed, "state")} removed${stateNames(changes.removed)}` : null,
              ]
                .filter(Boolean)
                .join(" · ")}
            </span>
            <button type="button" className="btn btn-sm" disabled={!keyOk} onClick={() => setMapOpen(true)}>
              Review mapping
            </button>
          </div>
        )}

        <Field label="Auto-import">
          <div className="pv-inline">
            <Switch
              checked={link.autoImport}
              label="Auto-import"
              disabled={busy === "patch"}
              onChange={(v) => void patch({ autoImport: v })}
            />
            <span className="pv-hint">
              {link.autoImport
                ? "New issues in this scope are imported automatically."
                : "Import manually from the board."}
            </span>
          </div>
          {link.autoImport && !hasRepoRoute && (
            <span className="pv-hint pv-hint-warn">Auto-import needs a default repo or a routing rule.</span>
          )}
        </Field>

        <Field label="Default repo for imports">
          <RepoChoice
            repos={repos}
            value={link.defaultRepoId}
            allowNone
            disabled={busy === "patch"}
            label="Default repo for imports"
            onChange={(v) => void patch({ defaultRepoId: v })}
          />
        </Field>

        <Field label="Routing rules" top>
          <RoutingRules
            link={link}
            repos={repos}
            disabled={busy === "patch"}
            keyOk={keyOk}
          />
        </Field>

        <Field label="Status mapping" top>
          {!mapOpen ? (
            <div className="pv-inline">
              {pending ? (
                <span className="pv-tag pv-tag-warn">Mapping pending</span>
              ) : (
                <span className="pv-hint">
                  Confirmed {formatAgo(link.stateMap.confirmedAt ?? 0)} ·{" "}
                  {plural(Object.keys(link.stateMap.pull).length, `${prov} state`)} mapped
                </span>
              )}
              {pending && <span className="pv-hint">Nodal won't push status changes until you confirm it.</span>}
              <button
                type="button"
                className={`btn btn-sm${pending ? " btn-primary" : ""}`}
                disabled={!keyOk}
                onClick={() => setMapOpen(true)}
              >
                {pending ? "Review mapping" : "Edit mapping"}
              </button>
            </div>
          ) : states.error ? (
            <div className="pv-alert pv-alert-danger" role="alert">
              <span className="pv-alert-text">Could not load states from {prov}. {states.error}</span>
              <button type="button" className="btn btn-sm" onClick={states.reload}>
                Retry
              </button>
            </div>
          ) : !report ? (
            <span className="pv-hint">Loading states from {prov}…</span>
          ) : (
            <StateMapEditor
              link={link}
              report={report}
              onCancel={() => setMapOpen(false)}
              onSaved={() => {
                setMapOpen(false);
                states.reload();
              }}
            />
          )}
        </Field>
      </div>
    </article>
  );
}

/** `: "QA", "Staging"` (up to 3 names). */
function stateNames(states: ExternalState[]): string {
  if (!states.length) return "";
  const names = states.slice(0, 3).map((s) => `"${s.name}"`);
  return `: ${names.join(", ")}${states.length > 3 ? "…" : ""}`;
}

function SourceStatus({
  keyOk,
  keyLoading,
  pending,
  drift,
  syncedAt,
  syncFailed,
  now,
}: {
  keyOk: boolean;
  keyLoading: boolean;
  pending: boolean;
  drift: number;
  syncedAt: number | null;
  syncFailed: boolean;
  now: number;
}) {
  if (keyLoading) return <span className="pv-conn pv-conn-muted">Checking…</span>;
  if (!keyOk)
    return (
      <span className="pv-conn pv-conn-danger">
        <span className="dot dot-sm" aria-hidden />
        Error
      </span>
    );
  return (
    <>
      {syncFailed ? (
        <span className="pv-conn pv-conn-danger">
          <span className="dot dot-sm" aria-hidden />
          Sync failed
        </span>
      ) : (
        <span className="pv-conn pv-conn-ok">
          <span className="dot dot-sm" aria-hidden />
          {syncedAt ? `Synced ${formatAgo(syncedAt, now)}` : "Not synced yet"}
        </span>
      )}
      {pending && <span className="pv-tag pv-tag-warn">Mapping pending</span>}
      {!pending && drift > 0 && <span className="pv-tag pv-tag-warn">Review mapping</span>}
    </>
  );
}

// ---------- Default repo and rules ----------

function RepoChoice({
  repos,
  value,
  onChange,
  allowNone,
  disabled,
  label,
}: {
  repos: Repo[];
  value: string | null;
  onChange: (v: string | null) => void;
  allowNone?: boolean;
  disabled?: boolean;
  label: string;
}) {
  if (repos.length === 0) return <span className="pv-hint">This project has no repos yet. Add one in Repos.</span>;
  if (repos.length <= 4) {
    const opts = [...(allowNone ? [{ value: "", label: "None" }] : []), ...repos.map((r) => ({ value: r.id, label: r.name }))];
    return (
      <Segmented value={value ?? ""} options={opts} label={label} disabled={disabled} onChange={(v) => onChange(v || null)} />
    );
  }
  return (
    <select
      className="input pv-select"
      aria-label={label}
      value={value ?? ""}
      disabled={disabled}
      onChange={(e) => onChange(e.target.value || null)}
    >
      {allowNone && <option value="">None</option>}
      {repos.map((r) => (
        <option key={r.id} value={r.id}>
          {r.name}
        </option>
      ))}
    </select>
  );
}

function RoutingRules({
  link,
  repos,
  disabled,
  keyOk,
}: {
  link: SourceLink;
  repos: Repo[];
  disabled?: boolean;
  /** Without a valid key, the provider's projects aren't fetched. */
  keyOk: boolean;
}) {
  const rules = link.repoRules;
  const saver = useRuleBackfill();
  /** `project` with `editing`: id of the rule being replaced (`null` = new). */
  const [form, setForm] = useState<{ kind: "label" } | { kind: "project"; editing: string | null } | null>(null);
  const [label, setLabel] = useState("");
  const [projectId, setProjectId] = useState("");
  const [repoId, setRepoId] = useState("");
  const projects = useRuleProjects([link.id], keyOk && form?.kind === "project");
  const repoName = (id: string) => repos.find((r) => r.id === id)?.name ?? "Unknown repo";
  const target = repoId || repos[0]?.id || "";
  const off = disabled || saver.busy;
  const canProjects = link.provider === "linear";

  const dup =
    form?.kind === "label" &&
    rules.some((r) => r.kind === "label" && r.value.trim().toLowerCase() === label.trim().toLowerCase());
  const editing = form?.kind === "project" ? form.editing : null;
  // A project goes to a single repo: exclude those that already have a rule (except the one being edited).
  const taken = new Set(rules.filter((r) => r.kind === "project" && r.id !== editing).map((r) => r.value));
  const options = projects.data?.filter((p) => !taken.has(p.project.id)).map((p) => p.project) ?? null;
  const project = options?.find((p) => p.id === projectId) ?? null;

  const open = (f: NonNullable<typeof form>, rule?: RepoRule) => {
    setForm(f);
    setLabel("");
    setProjectId(rule?.value ?? "");
    setRepoId(rule?.repoId ?? "");
  };

  const addLabel = async () => {
    const value = label.trim();
    if (!value || !target || dup) return;
    const rule: RepoRule = { id: "", kind: "label", value, name: value, repoId: target, createdAt: 0 };
    const same = (r: RepoRule) => r.kind === "label" && r.value.trim().toLowerCase() === value.toLowerCase();
    const ok = await saver.save({ link, update: (rs) => (rs.some(same) ? rs : [...rs, rule]) });
    if (ok) {
      setLabel("");
      setForm(null);
    }
  };

  const saveProject = async () => {
    if (form?.kind !== "project" || !project || !target) return;
    const prev = rules.find((r) => r.id === form.editing);
    if (prev && prev.value === project.id && prev.repoId === target) {
      setForm(null);
      return;
    }
    // New or changed: `id: ""` (the backend treats it as new) and it's replaced in place.
    const rule = newProjectRule(project, target);
    const editingId = form.editing;
    const update = (rs: RepoRule[]) =>
      editingId && rs.some((r) => r.id === editingId) ? rs.map((r) => (r.id === editingId ? rule : r)) : [...rs, rule];
    const backfill = { projectId: project.id, projectName: project.name, repoId: target, repoName: repoName(target) };
    if (await saver.save({ link, update }, backfill)) setForm(null);
  };

  const remove = (rule: RepoRule) =>
    void saver.save({
      link,
      update: (rs) => rs.filter((r) => (rule.id ? r.id !== rule.id : !(r.kind === rule.kind && r.value === rule.value))),
    });

  return (
    <div className="pv-rules">
      {rules.map((r, i) =>
        r.kind === "project" && editing === r.id ? null : (
          <div key={r.id || `${r.kind}-${r.value}-${i}`} className="pv-rule">
            <span className="pv-rule-label mono">
              {r.kind}: {r.kind === "project" ? r.name || r.value : r.value}
            </span>
            <span className="pv-map-arrow" aria-hidden>
              →
            </span>
            <span>{repoName(r.repoId)}</span>
            {r.kind === "project" && (
              <button
                type="button"
                className="btn btn-ghost btn-sm"
                aria-label={`Edit rule project ${r.name}`}
                disabled={off || form !== null}
                onClick={() => open({ kind: "project", editing: r.id }, r)}
              >
                Edit
              </button>
            )}
            <button
              type="button"
              className="icon-btn pv-rule-rm"
              aria-label={`Remove rule ${r.kind} ${r.kind === "project" ? r.name : r.value}`}
              disabled={off}
              onClick={() => remove(r)}
            >
              ✕
            </button>
          </div>
        ),
      )}
      {rules.length === 0 && !form && (
        <span className="pv-hint">
          {canProjects
            ? "Issues in a Linear project or with a label go to a specific repo. Project rules win over label rules; among labels, first match wins."
            : "Issues with a label go to a specific repo. First match wins."}
        </span>
      )}
      {form?.kind === "label" && (
        <form
          className="pv-rule pv-rule-form"
          onSubmit={(e) => {
            e.preventDefault();
            void addLabel();
          }}
        >
          <input
            className="input pv-rule-input"
            placeholder="Label"
            aria-label="Label"
            value={label}
            autoFocus
            onChange={(e) => setLabel(e.target.value)}
          />
          <span className="pv-map-arrow" aria-hidden>
            →
          </span>
          <RuleRepoSelect repos={repos} value={target} onChange={setRepoId} />
          <button type="submit" className="btn btn-sm" disabled={!label.trim() || !target || dup || off}>
            Add
          </button>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setForm(null)}>
            Cancel
          </button>
          {dup && <span className="pv-hint pv-hint-warn">There is already a rule for that label.</span>}
        </form>
      )}
      {form?.kind === "project" && (
        <form
          className="pv-rule pv-rule-form"
          onSubmit={(e) => {
            e.preventDefault();
            void saveProject();
          }}
        >
          <select
            className="input pv-select"
            aria-label="Linear project"
            value={project ? projectId : ""}
            disabled={!options}
            autoFocus
            onChange={(e) => setProjectId(e.target.value)}
          >
            <option value="">{projects.loading && !options ? "Loading projects…" : "Pick a project"}</option>
            {options?.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
          <span className="pv-map-arrow" aria-hidden>
            →
          </span>
          <RuleRepoSelect repos={repos} value={target} onChange={setRepoId} />
          <button type="submit" className="btn btn-sm" disabled={!project || !target || off}>
            {form.editing ? "Save" : "Add"}
          </button>
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => setForm(null)}>
            Cancel
          </button>
          {projects.error && (
            <span className="pv-hint pv-hint-error">
              Couldn't load Linear projects. {projects.error}{" "}
              <button type="button" className="btn btn-ghost btn-sm" onClick={projects.reload}>
                Retry
              </button>
            </span>
          )}
          {!keyOk && <span className="pv-hint pv-hint-warn">Linear isn't reachable with the saved key; projects can't be loaded.</span>}
          {options && options.length === 0 && (
            <span className="pv-hint">Every project in this team already has a rule.</span>
          )}
        </form>
      )}
      {!form && (
        <div className="pv-inline">
          {canProjects && (
            <button
              type="button"
              className="btn btn-ghost btn-sm pv-rule-add"
              disabled={off || repos.length === 0}
              onClick={() => open({ kind: "project", editing: null })}
            >
              + Project rule
            </button>
          )}
          <button
            type="button"
            className="btn btn-ghost btn-sm pv-rule-add"
            disabled={off || repos.length === 0}
            onClick={() => open({ kind: "label" })}
          >
            + Label rule
          </button>
        </div>
      )}
    </div>
  );
}

function RuleRepoSelect({ repos, value, onChange }: { repos: Repo[]; value: string; onChange: (id: string) => void }) {
  return (
    <select className="input pv-select" aria-label="Repo" value={value} onChange={(e) => onChange(e.target.value)}>
      {repos.map((r) => (
        <option key={r.id} value={r.id}>
          {r.name}
        </option>
      ))}
    </select>
  );
}
