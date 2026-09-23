import type { Viewer } from "../features/linear/api";
import type { RunView } from "../features/runs/status";
import { BrandMark } from "../ui/BrandMark";
import { Avatar } from "../features/linear/Board";

export type NavView = "board" | "runs" | "settings";

interface Props {
  current: NavView;
  counts: Partial<Record<NavView, number>>;
  activeRuns: RunView[];
  viewer: Viewer | null;
  linear: { ok: boolean; label: string };
  onNav: (v: NavView) => void;
  onOpenRun: (key: string) => void;
  onOpenPalette: () => void;
}

const NAV: { id: NavView; label: string; shortcut: string }[] = [
  { id: "board", label: "Board", shortcut: "⌘1" },
  { id: "runs", label: "Runs", shortcut: "⌘2" },
  { id: "settings", label: "Settings", shortcut: "⌘," },
];

export function Sidebar(p: Props) {
  return (
    <aside className="sidebar">
      <div className="brand">
        <BrandMark />
        <span className="brand-name">Agent Desk</span>
      </div>
      <div className="side-search-wrap">
        <button type="button" className="side-search" onClick={p.onOpenPalette} aria-keyshortcuts="Meta+K">
          <span>Search or run…</span>
          <kbd>⌘K</kbd>
        </button>
      </div>
      <nav className="side-nav" aria-label="Main">
        {NAV.map((n) => (
          <button
            key={n.id}
            type="button"
            className={`side-nav-item ${p.current === n.id ? "on" : ""}`}
            aria-current={p.current === n.id ? "page" : undefined}
            title={`${n.label} (${n.shortcut})`}
            onClick={() => p.onNav(n.id)}
          >
            <span className="side-nav-label">{n.label}</span>
            {p.counts[n.id] != null && <span className="side-nav-count num">{p.counts[n.id]}</span>}
          </button>
        ))}
      </nav>
      <h2 className="side-heading">Active runs</h2>
      <div className="side-runs">
        {p.activeRuns.map((r) => (
          <button key={r.key} type="button" className={`side-run tone-${r.tone}`} onClick={() => p.onOpenRun(r.key)}>
            <span className="dot pulse" aria-hidden />
            <span className="side-run-text">
              <span className="side-run-issue">{r.identifier ?? r.run?.name ?? r.runId}</span>
              <span className="side-run-label ellipsis">{r.label}</span>
            </span>
          </button>
        ))}
        {p.activeRuns.length === 0 && <div className="side-runs-empty">Nothing running.</div>}
      </div>
      <div className="side-foot">
        <Avatar name={p.viewer?.name} size="md" />
        <span className="side-foot-text">
          <span className="ellipsis">{p.viewer?.name ?? "Linear"}</span>
          <span className={`side-foot-status tone-${p.linear.ok ? "ok" : "danger"}`}>
            <span className="dot dot-sm" aria-hidden />
            {p.linear.label}
          </span>
        </span>
      </div>
    </aside>
  );
}
