import type { Project } from "../domain/types";

interface Props {
  project: Project | null;
  page: string;
  running: number;
  concurrency: number | null;
  needYou: number;
  queued: number;
  showImport: boolean;
  showNewTask: boolean;
  onOpenRuns: () => void;
  onImport: () => void;
  onNewTask: () => void;
}

export function Topbar(p: Props) {
  const runningLabel = p.concurrency != null ? `${p.running}/${p.concurrency} running` : `${p.running} running`;
  const pillLabel = [runningLabel, p.needYou ? `${p.needYou} need you` : "", p.queued ? `${p.queued} queued` : ""]
    .filter(Boolean)
    .join(", ");
  return (
    <header className="topbar">
      <nav className="crumb" aria-label="Breadcrumb">
        {p.project && (
          <>
            <span className="project-dot" style={{ ["--project-color" as string]: p.project.color }} aria-hidden />
            <span className="crumb-project ellipsis">{p.project.name}</span>
            <span className="crumb-sep" aria-hidden>
              /
            </span>
          </>
        )}
        <h1 className="crumb-page" aria-current="page">
          {p.page}
        </h1>
      </nav>
      <span className="spacer" />
      <button type="button" className="status-pill" onClick={p.onOpenRuns} aria-label={`${pillLabel}. Open runs`}>
        <span className="status-pill-running">
          <span className={`dot dot-sm tone-accent ${p.running ? "pulse" : ""}`} aria-hidden />
          {runningLabel}
        </span>
        {p.needYou > 0 && (
          <span className="status-pill-att">
            <span className="dot dot-sm tone-warn" aria-hidden />
            {p.needYou} need you
          </span>
        )}
        {p.queued > 0 && <span className="status-pill-queued">{p.queued} queued</span>}
      </button>
      {p.showImport && (
        <button type="button" className="btn btn-sm topbar-btn" onClick={p.onImport}>
          Import
        </button>
      )}
      {p.showNewTask && (
        <button type="button" className="btn btn-primary btn-sm topbar-btn" onClick={p.onNewTask}>
          New task
        </button>
      )}
    </header>
  );
}
