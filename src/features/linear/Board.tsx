import { useMemo, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { resolveRepo, type AppConfig, type Issue, type LinearError } from "./api";
import { COLUMNS, groupByColumn, PRIORITY_LABELS } from "./columns";
import { launchIssueRun } from "../runs/api";
import { issueRunBadge, type RunBadge } from "../runs/status";
import type { WorkflowInfo } from "../runs/types";
import { findRun, type RunsState } from "../runs/useRuns";
import { pickWorkflow, readLastWorkflow, useWorkflows, writeLastWorkflow } from "../runs/useWorkflows";
import "../runs/runs.css";

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

interface IssueCardProps {
  issue: Issue;
  repo: string | null;
  catalog: WorkflowInfo[];
  lastWorkflow: string;
  onPickWorkflow: (name: string) => void;
  badge: RunBadge | null;
  onLaunched: () => void;
  onOpenRun: (issueId: string) => void;
}

function IssueCard({ issue, repo, catalog, lastWorkflow, onPickWorkflow, badge, onLaunched, onOpenRun }: IssueCardProps) {
  const who = issue.assignee?.displayName || issue.assignee?.name;
  // null = seguir la última elección global.
  const [picked, setPicked] = useState<string | null>(null);
  const [launching, setLaunching] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const workflow = pickWorkflow(catalog, picked ?? lastWorkflow);
  // Si el catálogo no cargó (o falló) igual se puede lanzar: el backend valida el nombre.
  const options = catalog.length ? catalog : [{ name: workflow, description: null, whenToUse: null }];
  const blocked = !repo
    ? "Mapeá un repo para este team/proyecto en Ajustes"
    : badge?.active
      ? "Ya hay un run activo para esta issue"
      : null;

  const onRun = async () => {
    setLaunching(true);
    setError(null);
    try {
      await launchIssueRun({
        issueId: issue.id,
        identifier: issue.identifier,
        teamId: issue.team.id,
        projectId: issue.project?.id ?? null,
        workflow,
      });
      onLaunched();
    } catch (e) {
      setError(String(e));
    } finally {
      setLaunching(false);
    }
  };

  return (
    <article className="card">
      <button
        type="button"
        className="card-open"
        onClick={() => openUrl(issue.url).catch((e) => console.error("openUrl", e))}
        title={`Abrir ${issue.identifier} en Linear`}
      >
        <span className="card-top">
          <PriorityIcon priority={issue.priority} />
          <span className="card-id">{issue.identifier}</span>
          <span
            className={`repo-flag ${repo ? "has-repo" : "no-repo"}`}
            title={repo ? `Repo: ${repo}` : "Team/proyecto sin repo mapeado (Ajustes)"}
          >
            {repo ? "repo" : "sin repo"}
          </span>
        </span>
        <span className="card-title">{issue.title}</span>
      </button>
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
      <div className="card-actions">
        <select
          className="card-workflow"
          aria-label={`Workflow para ${issue.identifier}`}
          value={workflow}
          disabled={launching}
          onChange={(e) => {
            setPicked(e.target.value);
            onPickWorkflow(e.target.value);
          }}
        >
          {options.map((w) => (
            <option key={w.name} value={w.name} title={w.description ?? w.whenToUse ?? undefined}>
              {w.name}
            </option>
          ))}
        </select>
        {/* El span lleva el tooltip: un botón disabled no dispara hover en WebKit. */}
        <span title={blocked ?? `Lanzar ${workflow} para ${issue.identifier}`}>
          <button type="button" className="btn btn-sm" onClick={onRun} disabled={launching || blocked !== null}>
            {launching ? "Lanzando…" : "Run"}
          </button>
        </span>
        {badge && (
          <button
            type="button"
            className={`run-badge tone-${badge.tone}`}
            title={badge.title}
            onClick={() => onOpenRun(issue.id)}
          >
            {badge.label}
          </button>
        )}
      </div>
      {error && <p className="card-error">{error}</p>}
    </article>
  );
}

interface BoardViewProps {
  issues: Issue[];
  config: AppConfig;
  runs: RunsState;
  onOpenRun: (issueId: string) => void;
}

export function BoardView({ issues, config, runs, onOpenRun }: BoardViewProps) {
  const grouped = useMemo(() => groupByColumn(issues), [issues]);
  const columns = COLUMNS.filter((c) => !c.hideWhenEmpty || grouped.get(c.id)!.length > 0);
  const catalogs = useWorkflows(config.repos.map((r) => r.path));
  const [lastWorkflow, setLastWorkflow] = useState(readLastWorkflow);
  const pickLast = (name: string) => {
    setLastWorkflow(name);
    writeLastWorkflow(name);
  };

  const badgeFor = (issueId: string): RunBadge | null => {
    const ir = runs.byIssue.get(issueId);
    if (!ir) return null;
    const run = findRun(runs.runs, ir);
    return issueRunBadge(ir, run, run ? runs.details[run.sessionId] : undefined);
  };

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
                list.map((issue) => {
                  const repo = resolveRepo(config, issue.team.id, issue.project?.id);
                  return (
                    <IssueCard
                      key={issue.id}
                      issue={issue}
                      repo={repo}
                      catalog={(repo && catalogs[repo]) || catalogs[""] || []}
                      lastWorkflow={lastWorkflow}
                      onPickWorkflow={pickLast}
                      badge={badgeFor(issue.id)}
                      onLaunched={() => void runs.refresh()}
                      onOpenRun={onOpenRun}
                    />
                  );
                })
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
