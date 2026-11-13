import type { Project } from "../domain/types";
import { projectColor } from "../features/projects/create";

interface Props {
  project: Project | null;
  page: string;
  showImport: boolean;
  showNewTask: boolean;
  onImport: () => void;
  onNewTask: () => void;
}

export function Topbar(p: Props) {
  return (
    <header className="topbar">
      <nav className="crumb" aria-label="Breadcrumb">
        {p.project && (
          <>
            <span className="project-dot" style={{ ["--project-color" as string]: projectColor(p.project.color) }} aria-hidden />
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
      {p.showImport && (
        <button type="button" className="btn" onClick={p.onImport}>
          Import
        </button>
      )}
      {p.showNewTask && (
        <button type="button" className="btn btn-primary" onClick={p.onNewTask}>
          New task
        </button>
      )}
    </header>
  );
}
