import { useMemo } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { resolveRepo, type AppConfig, type Issue, type LinearError } from "./api";
import { COLUMNS, groupByColumn, PRIORITY_LABELS } from "./columns";

function PriorityIcon({ priority }: { priority: number }) {
  if (priority === 1) return <span className="prio prio-urgent" aria-hidden>!</span>;
  // 3 barras: Alta = 3 llenas, Media = 2, Baja = 1, sin prioridad = 0.
  const filled = priority === 0 ? 0 : 5 - priority;
  return (
    <span className="prio" aria-hidden>
      {[1, 2, 3].map((i) => (
        <i key={i} className={i <= filled ? "on" : ""} style={{ height: 3 + i * 3 }} />
      ))}
    </span>
  );
}

function IssueCard({ issue, repo }: { issue: Issue; repo: string | null }) {
  const who = issue.assignee?.displayName || issue.assignee?.name;
  return (
    <button
      type="button"
      className="card"
      onClick={() => openUrl(issue.url).catch((e) => console.error("openUrl", e))}
      title={`Abrir ${issue.identifier} en Linear`}
    >
      <div className="card-top">
        <PriorityIcon priority={issue.priority} />
        <span className="card-id">{issue.identifier}</span>
        <span
          className={`repo-flag ${repo ? "has-repo" : "no-repo"}`}
          title={repo ? `Repo: ${repo}` : "Team/proyecto sin repo mapeado (Ajustes)"}
        >
          {repo ? "repo" : "sin repo"}
        </span>
      </div>
      <div className="card-title">{issue.title}</div>
      <div className="card-meta">
        <span className="state-chip">
          <i style={{ background: issue.state.color }} />
          {issue.state.name}
        </span>
        <span className="chip">{issue.team.key}</span>
        {issue.project && <span className="chip muted-chip">{issue.project.name}</span>}
        {issue.parent && <span className="chip muted-chip">↳ {issue.parent.identifier}</span>}
        <span className="sr-only">Prioridad: {PRIORITY_LABELS[issue.priority] ?? issue.priorityLabel}</span>
        <span className="assignee" title={who ?? "Sin asignar"}>
          {who ? <span className="avatar">{who.slice(0, 1).toUpperCase()}</span> : <span className="avatar empty" />}
        </span>
      </div>
    </button>
  );
}

export function BoardView({ issues, config }: { issues: Issue[]; config: AppConfig }) {
  const grouped = useMemo(() => groupByColumn(issues), [issues]);
  const columns = COLUMNS.filter((c) => !c.hideWhenEmpty || grouped.get(c.id)!.length > 0);

  return (
    <main className="board">
      {columns.map((col) => {
        const list = grouped.get(col.id)!;
        return (
          <section key={col.id} className="column">
            <h2>
              {col.label} <span className="count">{list.length}</span>
            </h2>
            <div className="cards">
              {list.length === 0 ? (
                <p className="empty">Sin issues</p>
              ) : (
                list.map((issue) => (
                  <IssueCard
                    key={issue.id}
                    issue={issue}
                    repo={resolveRepo(config, issue.team.id, issue.project?.id)}
                  />
                ))
              )}
            </div>
          </section>
        );
      })}
    </main>
  );
}

const ERROR_TITLES: Record<LinearError["kind"], string> = {
  missingKey: "Falta la API key de Linear",
  invalidKey: "La API key no es válida",
  network: "Sin conexión con Linear",
  rateLimited: "Demasiadas peticiones",
  keychain: "No se pudo acceder al llavero",
  api: "Linear devolvió un error",
  unknown: "Algo salió mal",
};

export function ErrorState({
  error,
  onRetry,
  onSettings,
}: {
  error: LinearError;
  onRetry: () => void;
  onSettings: () => void;
}) {
  const keyProblem = error.kind === "invalidKey" || error.kind === "missingKey";
  return (
    <div className="state-panel" role="alert">
      <h2>{ERROR_TITLES[error.kind]}</h2>
      <p>{error.message}</p>
      <div className="actions">
        {keyProblem ? (
          <button className="btn primary" onClick={onSettings}>Ir a Ajustes</button>
        ) : (
          <button className="btn primary" onClick={onRetry}>Reintentar</button>
        )}
      </div>
    </div>
  );
}
