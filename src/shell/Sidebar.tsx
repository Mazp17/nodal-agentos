import type { Project } from "../domain/types";
import { BrandMark } from "../ui/BrandMark";
import { Kbd, KbdGroup } from "../ui/Kbd";
import type { ProjectPage, Route } from "./useNav";

type IconName = "board" | "chat" | "runs" | "settings";

const PROJECT_PAGES: { page: ProjectPage; label: string; icon: IconName; keys?: string[] }[] = [
  { page: "board", label: "Tasks", icon: "board", keys: ["⌘", "1"] },
  { page: "chat", label: "Chat", icon: "chat", keys: ["⌘", "2"] },
  { page: "runs", label: "Runs", icon: "runs", keys: ["⌘", "3"] },
  { page: "project-settings", label: "Settings", icon: "settings" },
];

const GEAR =
  "M5.9 1.5h2.2l.35 1.55c.4.15.77.36 1.1.62l1.52-.48 1.1 1.9-1.17 1.08a4 4 0 0 1 0 1.26l1.17 1.08-1.1 1.9-1.52-.48c-.33.26-.7.47-1.1.62L8.1 12.5H5.9l-.35-1.55a4 4 0 0 1-1.1-.62l-1.52.48-1.1-1.9 1.17-1.08a4 4 0 0 1 0-1.26L1.83 5.49l1.1-1.9 1.52.48c.33-.26.7-.47 1.1-.62z";

/** 14px line icons from the Nodal design. */
function SideIcon({ name }: { name: IconName }) {
  return (
    <svg
      className="side-icon"
      width="14"
      height="14"
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinejoin="round"
      strokeLinecap="round"
      aria-hidden
      focusable="false"
    >
      {name === "board" && (
        <>
          <rect x="1.5" y="2" width="3" height="10" rx="1" />
          <rect x="5.5" y="2" width="3" height="7" rx="1" />
          <rect x="9.5" y="2" width="3" height="4.5" rx="1" />
        </>
      )}
      {name === "chat" && (
        <path d="M3.5 2h7A1.5 1.5 0 0 1 12 3.5v5A1.5 1.5 0 0 1 10.5 10H6.5L4 12v-2h-.5A1.5 1.5 0 0 1 2 8.5v-5A1.5 1.5 0 0 1 3.5 2z" />
      )}
      {name === "runs" && (
        <>
          <circle cx="7" cy="7" r="5.5" />
          <path d="M5.9 4.9l3.2 2.1-3.2 2.1z" fill="currentColor" stroke="none" />
        </>
      )}
      {name === "settings" && (
        <>
          <path d={GEAR} />
          <circle cx="7" cy="7" r="1.8" />
        </>
      )}
    </svg>
  );
}

function Keys({ keys }: { keys: string[] }) {
  return (
    <KbdGroup aria-hidden>
      {keys.map((k) => (
        <Kbd key={k}>{k}</Kbd>
      ))}
    </KbdGroup>
  );
}

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
  /** Projects with a chat whose answer the user hasn't seen. */
  unreadChats: ReadonlySet<string>;
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
  onPickProject: (projectId: string) => void;
  onToggleProject: (projectId: string) => void;
  onNewProject: () => void;
  onOpenPalette: () => void;
}

export function Sidebar(p: Props) {
  const global = (page: "board" | "runs" | "settings") => p.current.projectId === null && p.current.page === page;
  const top: {
    page: "board" | "runs" | "settings";
    label: string;
    icon: IconName;
    count?: number;
    live?: boolean;
    keys?: string[];
  }[] = [
    { page: "board", label: "All projects", icon: "board", count: p.openTotal || undefined },
    { page: "runs", label: "Runs", icon: "runs", count: p.activeTotal || undefined, live: p.activeTotal > 0 },
    { page: "settings", label: "Settings", icon: "settings", keys: ["⌘", ","] },
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
      <button type="button" className="btn side-search" onClick={p.onOpenPalette} aria-keyshortcuts="Meta+K">
        <span className="ellipsis">Search or run…</span>
        <Keys keys={["⌘", "K"]} />
      </button>

      <nav className="side-nav" aria-label="Main">
        {top.map((n) => (
          <button
            key={n.page}
            type="button"
            className={`side-item ${global(n.page) ? "on" : ""}`}
            aria-current={global(n.page) ? "page" : undefined}
            aria-keyshortcuts={n.keys ? n.keys.join("+").replace("⌘", "Meta") : undefined}
            onClick={() => p.onGo(n.page, null)}
          >
            <SideIcon name={n.icon} />
            <span className="side-item-label ellipsis">{n.label}</span>
            {n.live && <span className="dot dot-sm pulse tone-accent" aria-hidden />}
            {n.count != null && (
              <span className="side-count num" aria-label={n.page === "runs" ? `${n.count} running` : `${n.count} open tasks`}>
                {n.count}
              </span>
            )}
            {n.keys && <Keys keys={n.keys} />}
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
          const unread = p.unreadChats.has(proj.id);
          return (
            <div key={proj.id} className="side-project" role="group" aria-label={proj.name}>
              <div className={`side-project-row ${cur && !open ? "on-soft" : ""}`}>
                <button
                  type="button"
                  className="side-project-pick"
                  aria-current={cur && !open ? "true" : undefined}
                  onClick={() => p.onPickProject(proj.id)}
                >
                  <span className="project-dot" style={{ ["--project-color" as string]: proj.color }} aria-hidden />
                  <span className="side-item-label ellipsis">{proj.name}</span>
                  {unread && !open && <span className="side-unread" role="img" aria-label="Unread chat" title="Unread chat" />}
                  {drift && (
                    <span
                      className="dot dot-sm tone-warn"
                      role="img"
                      aria-label="Source states changed: review the mapping"
                      title="Source states changed: review the mapping in Settings → Sources"
                    />
                  )}
                  {running > 0 && (
                    <span className="side-count num" title={`${running} running`} aria-label={`${running} running`}>
                      {running}
                    </span>
                  )}
                </button>
                <button
                  type="button"
                  className="icon-btn side-project-btn side-chev"
                  title="Show or hide pages"
                  aria-label={`Show ${proj.name} pages`}
                  aria-expanded={open}
                  onClick={() => p.onToggleProject(proj.id)}
                >
                  <span aria-hidden>{open ? "▾" : "▸"}</span>
                </button>
              </div>
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
                      <SideIcon name={sp.icon} />
                      <span className="side-item-label ellipsis">{sp.label}</span>
                      {sp.page === "chat" && unread && (
                        <span className="side-unread" role="img" aria-label="Unread chat" title="Unread chat" />
                      )}
                      {sp.page === "project-settings" && drift && (
                        <span className="dot dot-sm tone-warn" aria-label="Mapping to review" />
                      )}
                      {cur && sp.keys && <Keys keys={sp.keys} />}
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
