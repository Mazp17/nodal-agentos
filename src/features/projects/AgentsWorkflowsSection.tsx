import { useId, useMemo, useState } from "react";
import { listExecutors, type ExecutorInfo } from "../../domain/api";
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
import { Segmented, type SegOption } from "./fields";

type KindFilter = "all" | "agent" | "workflow";

interface Row {
  id: string;
  key: HiddenExecutor;
  kind: "agent" | "workflow";
  name: string;
  description: string | null;
  /** Repos whose own definition replaces this `~/.claude` one. */
  overriddenBy: string[];
  /** A repo definition that replaces a `~/.claude` one with the same name. */
  overridesUser: boolean;
}

interface Group {
  id: string;
  title: string;
  hint: string;
  rows: Row[];
}

const sameDef = (a: ExecutorInfo, kind: string, name: string) =>
  a.executor.kind === kind && a.executor.kind !== "claude" && a.executor.name === name;

function toRow(info: ExecutorInfo, repoId: string | null, extra: Partial<Row> = {}): Row | null {
  const key = hiddenKeyOf(info, repoId);
  if (!key) return null;
  return {
    id: hiddenId(key),
    key,
    kind: key.kind,
    name: key.name,
    description: info.description,
    overriddenBy: [],
    overridesUser: false,
    ...extra,
  };
}

/**
 * `catalogs[0]` is the repo-less catalog (`~/.claude` + user plugins), then one per repo in
 * `repos` order. `list_executors` drops a `~/.claude` entry a repo overrides, so the flags come
 * from comparing catalogs.
 */
function groupExecutors(repos: readonly Repo[], catalogs: readonly ExecutorInfo[][]): Group[] {
  const user = catalogs[0] ?? [];
  const perRepo = repos.map((r, i) => ({ repo: r, list: catalogs[i + 1] ?? [] }));
  const isRepoDef = (i: ExecutorInfo) => i.source === "repo" && i.executor.kind !== "claude";

  const repoGroups: Group[] = perRepo.map(({ repo, list }) => ({
    id: `repo:${repo.id}`,
    title: repo.name,
    hint: `${repo.path}/.claude`,
    rows: list
      .filter(isRepoDef)
      .map((info) => {
        const ex = info.executor as { kind: "agent" | "workflow"; name: string };
        const overridesUser = user.some((u) => u.source === "user" && sameDef(u, ex.kind, ex.name));
        return toRow(info, repo.id, { overridesUser });
      })
      .filter((r): r is Row => r !== null),
  }));

  const system: Group = {
    id: "user",
    title: "System",
    hint: "~/.claude",
    rows: user
      .filter((i) => i.source === "user")
      .map((info) => {
        const ex = info.executor as { kind: "agent" | "workflow"; name: string };
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
  const plugin: Group = { id: "plugin", title: "Plugin", hint: "Enabled Claude Code plugins", rows: [...plugins.values()] };

  // Busiest sources first; ties keep repo order, then System and Plugin.
  return [...repoGroups, system, plugin].sort((a, b) => b.rows.length - a.rows.length);
}

const useCatalogs = (repos: readonly Repo[]) => {
  const ids = repos.map((r) => r.id);
  // Under `repos` like the pickers' catalogs, so Rescan and backend notices refetch them together.
  return usePolled<ExecutorInfo[][]>(
    `executors-all:${ids.join(",")}`,
    () => Promise.all([null, ...ids].map((id) => listExecutors(id))),
    ["repos"],
    0,
  );
};

export function AgentsWorkflowsSection({ project }: { project: Project }) {
  const ctx = useProjects();
  const toast = useToast();
  const searchId = useId();
  const repos = ctx.reposOf(project.id);
  const catalogs = useCatalogs(repos);
  const hiddenKeys = useHiddenExecutors(project.id);
  const [kind, setKind] = useState<KindFilter>("all");
  const [query, setQuery] = useState("");
  const [scanning, setScanning] = useState(false);

  const hidden = useMemo(() => hiddenSetOf(hiddenKeys.data), [hiddenKeys.data]);
  const groups = useMemo(() => groupExecutors(repos, catalogs.data ?? []), [repos, catalogs.data]);

  const q = query.trim().toLowerCase();
  const matchesName = (r: Row) => !q || r.name.toLowerCase().includes(q);
  const named = groups.flatMap((g) => g.rows).filter(matchesName);
  const count = (k: KindFilter) => named.filter((r) => k === "all" || r.kind === k).length;
  const found = named.filter((r) => kind === "all" || r.kind === kind);
  const foundHidden = found.filter((r) => hidden.has(r.id)).length;
  const visible = (r: Row) => matchesName(r) && (kind === "all" || r.kind === kind);
  const claudeVisible = kind === "all" && (!q || "claude".includes(q));

  const kinds: SegOption<KindFilter>[] = [
    { value: "all", label: `All ${count("all")}` },
    { value: "agent", label: `Agents ${count("agent")}` },
    { value: "workflow", label: `Workflows ${count("workflow")}` },
  ];

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
        title="Agents & workflows"
        text="Turn off the ones this project doesn't use. They only disappear from Nodal's pickers in every repo of the project; their definition files are never touched."
      >
        <button type="button" className="btn" disabled={scanning} onClick={() => void rescan()}>
          {scanning ? "Rescanning…" : "Rescan"}
        </button>
      </SectionHead>

      <div className="aw-toolbar">
        <Segmented label="Kind" value={kind} options={kinds} onChange={setKind} />
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
        <span className="spacer" />
        <span className="aw-summary" aria-live="polite">
          {found.length} found · {foundHidden} hidden
        </span>
      </div>

      {catalogs.error && <div className="repo-notice">Couldn't read the agents and workflows: {catalogs.error}</div>}
      {hiddenKeys.error && <div className="repo-notice">Couldn't read what this project hides: {hiddenKeys.error}</div>}

      {claudeVisible && (
        <div className="panel settings-card">
          <div className="aw-row">
            <button type="button" role="switch" aria-checked className="switch" aria-label="Claude is always shown" disabled />
            <span className="aw-mark aw-mark-claude" title="Claude session" aria-hidden>
              C
            </span>
            <div className="aw-row-text">
              <span className="aw-row-name">Claude</span>
              <span className="aw-row-desc">Plain Claude Code session. Always shown; it can't be hidden.</span>
            </div>
          </div>
        </div>
      )}

      {loading && <p className="settings-note">Scanning agents and workflows…</p>}

      {!loading &&
        groups.map((g) => {
          const rows = g.rows.filter(visible);
          if (rows.length === 0 && (q || kind !== "all")) return null;
          const shown = g.rows.filter((r) => !hidden.has(r.id)).length;
          return (
            <section key={g.id} className="panel settings-card aw-group" aria-label={g.title}>
              <header className="aw-group-head">
                <span className="aw-group-title">{g.title}</span>
                <span className="aw-group-hint mono">{g.hint}</span>
                <span className="spacer" />
                <span className="aw-group-count">
                  {shown} of {g.rows.length} shown
                </span>
              </header>
              {rows.length === 0 ? (
                <div className="aw-empty">No agents or workflows here.</div>
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
  const isAgent = row.kind === "agent";
  const over = row.overriddenBy;
  return (
    <div className={`aw-row${hidden ? " aw-row-off" : ""}`}>
      <button
        type="button"
        role="switch"
        aria-checked={!hidden}
        aria-label={`Show ${row.name} in pickers`}
        className="switch"
        onClick={() => void onToggle(row, !hidden)}
      />
      <span className="aw-mark" title={isAgent ? "Agent" : "Workflow"}>
        {isAgent ? "A" : "W"}
      </span>
      <div className="aw-row-text">
        <span className="aw-row-name mono">{row.name}</span>
        {row.description && <span className="aw-row-desc">{row.description}</span>}
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
