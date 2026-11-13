import { useId, useMemo, useState } from "react";
import { listChatAgents, type ExecutorInfo } from "../../domain/api";
import { useProjects } from "../../domain/hooks/projects";
import {
  hiddenId,
  hiddenKeyOf,
  hiddenSetOf,
  invalidate,
  toggleHiddenExecutor,
  useHiddenExecutors,
  usePolled,
} from "../../domain/hooks/store";
import type { HiddenExecutor, Project, Repo } from "../../domain/types";
import { useToast } from "../../ui/Toasts";
import { SectionHead } from "../settings/SettingsView";

interface Row {
  id: string;
  key: HiddenExecutor;
  name: string;
  description: string | null;
  /** Repos whose own definition replaces this `~/.claude` one. */
  overriddenBy: string[];
  /** A repo definition that replaces a `~/.claude` one with the same name. */
  overridesUser: boolean;
}

interface Group {
  id: string;
  /** Where the definitions come from: a repo, `~/.claude` or a plugin. */
  source: "repo" | "user" | "plugin";
  title: string;
  hint: string;
  rows: Row[];
}

const SOURCE_TAG: Record<Group["source"], string> = { repo: "Repo", user: "User", plugin: "Plugin" };

const sameDef = (a: ExecutorInfo, kind: string, name: string) =>
  a.executor.kind === kind && a.executor.kind !== "claude" && a.executor.name === name;

function toRow(info: ExecutorInfo, repoId: string | null, extra: Partial<Row> = {}): Row | null {
  const key = hiddenKeyOf(info, repoId);
  if (!key) return null;
  return {
    id: hiddenId(key),
    key,
    name: key.name,
    description: info.description,
    overriddenBy: [],
    overridesUser: false,
    ...extra,
  };
}

/**
 * `catalogs[0]` is the repo-less catalog (`~/.claude` + user plugins), then one per repo in
 * `repos` order. `list_chat_agents` drops a `~/.claude` entry a repo overrides, so the flags come
 * from comparing catalogs.
 */
function groupExecutors(repos: readonly Repo[], catalogs: readonly ExecutorInfo[][]): Group[] {
  const user = catalogs[0] ?? [];
  const perRepo = repos.map((r, i) => ({ repo: r, list: catalogs[i + 1] ?? [] }));
  const isRepoDef = (i: ExecutorInfo) => i.source === "repo" && i.executor.kind === "agent";

  const repoGroups: Group[] = perRepo.map(({ repo, list }) => ({
    id: `repo:${repo.id}`,
    source: "repo",
    title: repo.name,
    hint: `${repo.path}/.claude`,
    rows: list
      .filter(isRepoDef)
      .map((info) => {
        const ex = info.executor as { kind: "agent"; name: string };
        const overridesUser = user.some((u) => u.source === "user" && sameDef(u, ex.kind, ex.name));
        return toRow(info, repo.id, { overridesUser });
      })
      .filter((r): r is Row => r !== null),
  }));

  const system: Group = {
    id: "user",
    source: "user",
    title: "System",
    hint: "~/.claude",
    rows: user
      .filter((i) => i.source === "user")
      .map((info) => {
        const ex = info.executor as { kind: "agent"; name: string };
        const overriddenBy = perRepo
          .filter(({ list }) => list.some((i) => isRepoDef(i) && sameDef(i, ex.kind, ex.name)))
          .map(({ repo }) => repo.name);
        return toRow(info, null, { overriddenBy });
      })
      .filter((r): r is Row => r !== null),
  };

  const plugins = new Map<string, Row>();
  for (const list of catalogs) {
    for (const info of list) {
      if (info.source !== "plugin") continue;
      const row = toRow(info, null);
      if (row && !plugins.has(row.id)) plugins.set(row.id, row);
    }
  }
  const plugin: Group = { id: "plugin", source: "plugin", title: "Enabled plugins", hint: "~/.claude/plugins", rows: [...plugins.values()] };

  // Busiest sources first; ties keep repo order, then System and Plugin.
  return [...repoGroups, system, plugin].sort((a, b) => b.rows.length - a.rows.length);
}

const useCatalogs = (projectId: string, repos: readonly Repo[]) => {
  const ids = repos.map((r) => r.id);
  // Under `repos` like the pickers' catalogs, so Rescan and backend notices refetch them together.
  return usePolled<ExecutorInfo[][]>(
    `agents-all:${projectId}:${ids.join(",")}`,
    () => Promise.all([null, ...ids].map((id) => listChatAgents(projectId, id))),
    ["repos"],
    0,
  );
};

export function AgentsSection({ project }: { project: Project }) {
  const ctx = useProjects();
  const toast = useToast();
  const searchId = useId();
  const repos = ctx.reposOf(project.id);
  const catalogs = useCatalogs(project.id, repos);
  const hiddenKeys = useHiddenExecutors(project.id);
  const [query, setQuery] = useState("");
  const [scanning, setScanning] = useState(false);

  const hidden = useMemo(() => hiddenSetOf(hiddenKeys.data), [hiddenKeys.data]);
  const groups = useMemo(() => groupExecutors(repos, catalogs.data ?? []), [repos, catalogs.data]);

  const q = query.trim().toLowerCase();
  const matchesName = (r: Row) => !q || r.name.toLowerCase().includes(q);
  const named = groups.flatMap((g) => g.rows).filter(matchesName);
  const found = named;
  const foundHidden = found.filter((r) => hidden.has(r.id)).length;
  const visible = matchesName;
  const claudeVisible = !q || "claude".includes(q);

  const rescan = async () => {
    setScanning(true);
    try {
      await invalidate("repos");
    } finally {
      setScanning(false);
    }
  };

  const toggle = async (row: Row, hide: boolean) => {
    try {
      await toggleHiddenExecutor(project.id, row.key, hide);
    } catch (e) {
      toast(`Couldn't ${hide ? "hide" : "show"} ${row.name}`, String(e), "danger");
    }
  };

  const loading = catalogs.data === undefined && !catalogs.error;

  return (
    <>
      <SectionHead
        title="Agents"
        text="Found in this project's repos and in your Claude config. Turn one off to hide it from the chat agent picker; the file is not touched."
      >
        <button type="button" className="btn" disabled={scanning} onClick={() => void rescan()}>
          {scanning ? "Rescanning…" : "Rescan"}
        </button>
      </SectionHead>

      <div className="aw-toolbar">
        <label className="sr-only" htmlFor={searchId}>
          Filter by name
        </label>
        <input
          id={searchId}
          type="search"
          className="input aw-search"
          placeholder="Filter by name"
          spellCheck={false}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
        <span className="aw-summary" aria-live="polite">
          {found.length} found · {foundHidden} hidden
        </span>
      </div>

      {catalogs.error && <div className="repo-notice">Couldn't read the agents: {catalogs.error}</div>}
      {hiddenKeys.error && <div className="repo-notice">Couldn't read what this project hides: {hiddenKeys.error}</div>}

      {claudeVisible && (
        <div className="panel settings-card">
          <div className="aw-row">
            <button type="button" role="switch" aria-checked className="switch" aria-label="Claude is always shown" disabled />
            <span className="aw-mark aw-mark-claude" title="Claude session" aria-hidden>
              C
            </span>
            <div className="aw-row-text">
              <span className="aw-row-name mono">Claude</span>
              <span className="aw-row-desc ellipsis">Plain Claude Code session. Always shown; it can't be hidden.</span>
            </div>
          </div>
        </div>
      )}

      {loading && <p className="settings-note">Scanning agents…</p>}

      {!loading && q && !claudeVisible && found.length === 0 && (
        <div className="aw-none">Nothing matches that filter.</div>
      )}

      {!loading &&
        groups.map((g) => {
          const rows = g.rows.filter(visible);
          if (rows.length === 0 && q) return null;
          const shown = g.rows.filter((r) => !hidden.has(r.id)).length;
          return (
            <section key={g.id} className="panel settings-card aw-group" aria-label={g.title}>
              <header className="aw-group-head">
                <span className={`aw-group-tag aw-group-tag-${g.source}`}>{SOURCE_TAG[g.source]}</span>
                <span className="aw-group-title">{g.title}</span>
                <span className="aw-group-hint mono ellipsis" title={g.hint}>
                  {g.hint}
                </span>
                <span className="aw-group-count">
                  {shown} of {g.rows.length} shown
                </span>
              </header>
              {rows.length === 0 ? (
                <div className="aw-empty">No agents here.</div>
              ) : (
                rows.map((r) => <ExecutorRow key={r.id} row={r} hidden={hidden.has(r.id)} onToggle={toggle} />)
              )}
            </section>
          );
        })}
    </>
  );
}

function ExecutorRow({ row, hidden, onToggle }: { row: Row; hidden: boolean; onToggle: (r: Row, hide: boolean) => Promise<void> }) {
  const over = row.overriddenBy;
  return (
    <div className={`aw-row${hidden ? " aw-row-off" : ""}`}>
      <button
        type="button"
        role="switch"
        aria-checked={!hidden}
        aria-label={`Show ${row.name} in the agent picker`}
        className="switch"
        onClick={() => void onToggle(row, !hidden)}
      />
      <span className="aw-mark aw-mark-agent" title="Agent">
        A
      </span>
      <div className="aw-row-text">
        <span className="aw-row-name mono ellipsis" title={row.name}>
          {row.name}
        </span>
        {row.description && (
          <span className="aw-row-desc ellipsis" title={row.description}>
            {row.description}
          </span>
        )}
      </div>
      <div className="aw-flags">
        {hidden && <span className="badge badge-sm tone-muted">hidden</span>}
        {over.length > 0 && (
          <span className="badge badge-sm tone-warn" title={`Replaced by the definition in ${over.join(", ")}`}>
            overridden by repo
          </span>
        )}
        {row.overridesUser && <span className="badge badge-sm tone-accent">overrides ~/.claude</span>}
      </div>
    </div>
  );
}
