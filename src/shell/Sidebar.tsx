import type { Project } from "../domain/types";
import { BrandMark } from "../ui/BrandMark";
import type { ProjectPage, Route } from "./useNav";

const PROJECT_PAGES: { page: ProjectPage; label: string }[] = [
  { page: "board", label: "Board" },
  { page: "tasks", label: "Tasks" },
  { page: "runs", label: "Runs" },
  { page: "activity", label: "Activity" },
  { page: "project-settings", label: "Settings" },
];

export interface ProviderFoot {
  tone: "ok" | "danger" | "muted";
  label: string;
}

interface Props {
  /** "Visible" route (on a run, the one it came from). */
  current: Route;
  projects: Project[];
  expanded: ReadonlySet<string>;
  openTotal: number;
  activeTotal: number;
  activeByProject: Map<string, number>;
  /** Projects with sources whose states changed in the provider (mapping to review). */
  mappingDrift: ReadonlySet<string>;
  provider: ProviderFoot;
  /** Installed version, and the newest one if the update check found one. */
  version: string | null;
  updateAvailable: string | null;
  updating: boolean;
  checkingUpdates: boolean;
  onUpdate: () => void;
  onGo: (page: "board" | "runs" | "settings" | ProjectPage, projectId: string | null) => void;
  onToggleProject: (projectId: string) => void;
  onNewProject: () => void;
  onOpenPalette: () => void;
}

export function Sidebar(p: Props) {
  const global = (page: "board" | "runs" | "settings") => p.current.projectId === null && p.current.page === page;
  const top: { page: "board" | "runs" | "settings"; label: string; count?: number; live?: boolean; kbd?: string }[] = [
    { page: "board", label: "All projects", count: p.openTotal || undefined },
    { page: "runs", label: "Runs", count: p.activeTotal || undefined, live: p.activeTotal > 0 },
    { page: "settings", label: "Settings", kbd: "⌘," },
  ];

  return (
    <aside className="sidebar" aria-label="Sidebar">
      <div className="brand">
        <BrandMark size={22} />
        <span className="brand-text">
          <span className="brand-name">Nodal</span>
          <span className="brand-tag">Agent OS for Claude</span>
        </span>
      </div>
      <button type="button" className="side-search" onClick={p.onOpenPalette} aria-keyshortcuts="Meta+K">
        <span>Search or run…</span>
        <kbd className="kbd">⌘K</kbd>
      </button>

      <nav className="side-nav" aria-label="Main">
        {top.map((n) => (
          <button
            key={n.page}
            type="button"
            className={`side-item ${global(n.page) ? "on" : ""}`}
            aria-current={global(n.page) ? "page" : undefined}
            title={n.kbd ? `${n.label} (${n.kbd})` : undefined}
            onClick={() => p.onGo(n.page, null)}
          >
            <span className="side-item-label">{n.label}</span>
            {n.live && <span className="dot dot-sm pulse tone-accent" aria-hidden />}
            {n.count != null && (
              <span className="side-count num" aria-label={n.page === "runs" ? `${n.count} running` : `${n.count} open tasks`}>
                {n.count}
              </span>
            )}
          </button>
        ))}
      </nav>

      <div className="side-heading">
        <h2 className="side-heading-label">Projects</h2>
        <button type="button" className="icon-btn side-add" title="New project" aria-label="New project" onClick={p.onNewProject}>
          +
        </button>
      </div>
      <nav className="side-projects" aria-label="Projects">
        {p.projects.map((proj) => {
          const open = p.expanded.has(proj.id);
          const cur = p.current.projectId === proj.id;
          const running = p.activeByProject.get(proj.id) ?? 0;
          const drift = p.mappingDrift.has(proj.id);
          return (
            <div key={proj.id} className="side-project" role="group" aria-label={proj.name}>
              <button
                type="button"
                className={`side-item side-project-head ${cur && !open ? "on-soft" : ""}`}
                aria-expanded={open}
                onClick={() => p.onToggleProject(proj.id)}
              >
                <span className="project-dot" style={{ ["--project-color" as string]: proj.color }} aria-hidden />
                <span className="side-item-label ellipsis">{proj.name}</span>
                {drift && (
                  <span
                    className="dot dot-sm tone-warn"
                    role="img"
                    aria-label="Source states changed: review the mapping"
                    title="Source states changed: review the mapping in Settings → Sources"
                  />
                )}
                {running > 0 && (
                  <span className="side-count side-count-live num" aria-label={`${running} running`}>
                    {running}
                  </span>
                )}
                <span className="side-chev" aria-hidden>
                  {open ? "▾" : "▸"}
                </span>
              </button>
              {open &&
                PROJECT_PAGES.map((sp) => {
                  const on = cur && p.current.page === sp.page;
                  return (
                    <button
                      key={sp.page}
                      type="button"
                      className={`side-item side-sub ${on ? "on" : ""}`}
                      aria-current={on ? "page" : undefined}
                      onClick={() => p.onGo(sp.page, proj.id)}
                    >
                      {sp.label}
                      {sp.page === "project-settings" && drift && (
                        <span className="dot dot-sm tone-warn side-sub-dot" aria-label="Mapping to review" />
                      )}
                    </button>
                  );
                })}
            </div>
          );
        })}
        {p.projects.length === 0 && <div className="side-empty">No projects yet.</div>}
      </nav>

      <div className="side-foot">
        <span className="side-foot-status" role="status">
          <span className={`dot dot-sm tone-${p.provider.tone}`} aria-hidden />
          <span className="ellipsis">{p.provider.label}</span>
        </span>
        {p.updateAvailable ? (
          <button
            type="button"
            className="side-version side-update"
            disabled={p.updating || p.checkingUpdates}
            onClick={p.onUpdate}
            aria-label={p.version ? `Update to v${p.updateAvailable} (installed v${p.version})` : undefined}
          >
            {p.updating ? "Updating…" : `Update to v${p.updateAvailable}`}
          </button>
        ) : (
          p.version && <span className="side-version">v{p.version}</span>
        )}
      </div>
    </aside>
  );
}
