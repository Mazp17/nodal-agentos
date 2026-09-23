import { useMemo } from "react";
import { resolveRepo, type AppConfig, type Issue, type LinearError } from "./api";
import { COLUMNS, groupByColumn, PRIORITY_LABELS } from "./columns";
import type { Launcher } from "../runs/actions";
import type { RunView } from "../runs/status";
import type { WorkflowInfo } from "../runs/types";
import { avatarHue, formatClock, initials } from "../../lib/format";
import "./board.css";

export function PriorityBars({ priority }: { priority: number }) {
  // Urgente: las 3 barras en rojo. Alta = 3, Media = 2, Baja = 1, sin prioridad = 0.
  const filled = priority === 0 ? 0 : 4 - priority;
  return (
    <span className="prio" title={PRIORITY_LABELS[priority]} aria-label={`Priority: ${PRIORITY_LABELS[priority] ?? "—"}`}>
      {[1, 2, 3].map((k) => (
        <i key={k} className={priority === 1 ? "urgent" : k <= filled ? "on" : ""} style={{ height: 2 + k * 3 }} />
      ))}
    </span>
  );
}

export function Avatar({ name, size = "sm" }: { name: string | null | undefined; size?: "sm" | "md" }) {
  if (!name) return <span className={`avatar avatar-${size} avatar-empty`} title="Unassigned" />;
  return (
    <span className={`avatar avatar-${size}`} style={{ ["--hue" as string]: avatarHue(name) }} title={name}>
      {initials(name)}
    </span>
  );
}

export function PhaseSegments({ view }: { view: RunView }) {
  if (view.kind !== "running" || view.phaseTotal === 0 || view.phaseIndex == null) return null;
  const cur = view.phaseIndex - 1;
  return (
    <div
      className={`segs tone-${view.tone}`}
      style={{ gridTemplateColumns: `repeat(${view.phaseTotal}, 1fr)` }}
      role="img"
      aria-label={view.label}
    >
      {Array.from({ length: view.phaseTotal }, (_, k) => (
        <span key={k} className={k < cur ? "on" : k === cur ? "on current" : ""} />
      ))}
    </div>
  );
}

export function RunBadge({ view, onClick }: { view: RunView; onClick?: () => void }) {
  const pulse = view.kind === "running" || view.kind === "starting";
  const body = (
    <>
      <span className={`dot dot-sm ${pulse ? "pulse" : ""}`} aria-hidden />
      {view.label}
    </>
  );
  return onClick ? (
    <button
      type="button"
      className={`badge tone-${view.tone}`}
      title={view.title}
      onClick={(e) => {
        e.stopPropagation();
        onClick();
      }}
    >
      {body}
    </button>
  ) : (
    <span className={`badge tone-${view.tone}`} title={view.title}>
      {body}
    </span>
  );
}

interface CardProps {
  issue: Issue;
  repo: string | null;
  view: RunView | undefined;
  selected: boolean;
  catalog: WorkflowInfo[];
  workflow: string;
  launching: boolean;
  onToggle: () => void;
  onOpen: () => void;
  onOpenRun: (key: string) => void;
  onPickWorkflow: (name: string) => void;
  onRun: () => void;
  onMapRepo: () => void;
}

function IssueCard(p: CardProps) {
  const { issue, repo, view } = p;
  const who = issue.assignee?.displayName || issue.assignee?.name;
  const canRun = repo !== null && !view?.active;
  const meta = [issue.team.key, issue.project?.name, issue.parent ? `↳ ${issue.parent.identifier}` : null]
    .filter(Boolean)
    .join(" · ");
  // Si el catálogo no cargó igual se puede lanzar: el backend valida el nombre.
  const options: { name: string; description: string | null }[] = p.catalog.length
    ? p.catalog
    : [{ name: p.workflow, description: null }];

  return (
    <article className={`card ${p.selected ? "card-selected" : ""}`} onClick={p.onOpen}>
      <div className="card-top">
        <button
          type="button"
          role="checkbox"
          aria-checked={p.selected}
          aria-label={`Select ${issue.identifier}`}
          className="card-check"
          onClick={(e) => {
            e.stopPropagation();
            p.onToggle();
          }}
        >
          {p.selected ? "✓" : ""}
        </button>
        <span className="card-id">{issue.identifier}</span>
        <PriorityBars priority={issue.priority} />
        <Avatar name={who} />
      </div>
      <button
        type="button"
        className="card-title"
        onClick={(e) => {
          e.stopPropagation();
          p.onOpen();
        }}
      >
        {issue.title}
      </button>
      <div className="card-meta">
        <span className="state-dot" style={{ color: issue.state.color }} aria-hidden />
        <span className="ellipsis">
          {issue.state.name} · {meta}
        </span>
      </div>
      {view && <PhaseSegments view={view} />}
      <div className="card-actions">
        {view && <RunBadge view={view} onClick={() => p.onOpenRun(view.key)} />}
        <span className="card-actions-end">
          {repo === null ? (
            <button
              type="button"
              className="btn btn-xs card-norepo"
              title="Map a repo for this team or project in Settings"
              onClick={(e) => {
                e.stopPropagation();
                p.onMapRepo();
              }}
            >
              No repo
            </button>
          ) : view?.active ? (
            <button
              type="button"
              className="btn btn-xs"
              onClick={(e) => {
                e.stopPropagation();
                p.onOpenRun(view.key);
              }}
            >
              Open run
            </button>
          ) : (
            <>
              <select
                className="card-workflow"
                aria-label={`Workflow for ${issue.identifier}`}
                title={options.find((w) => w.name === p.workflow)?.description ?? p.workflow}
                value={p.workflow}
                disabled={p.launching}
                onClick={(e) => e.stopPropagation()}
                onChange={(e) => p.onPickWorkflow(e.target.value)}
              >
                {options.map((w) => (
                  <option key={w.name} value={w.name}>
                    {w.name}
                  </option>
                ))}
              </select>
              <button
                type="button"
                className="btn btn-xs btn-run"
                disabled={p.launching || !canRun}
                title={`Launch ${p.workflow} for ${issue.identifier}`}
                onClick={(e) => {
                  e.stopPropagation();
                  p.onRun();
                }}
              >
                {p.launching ? "Launching…" : view?.kind === "failed" ? "Retry" : "Run"}
              </button>
            </>
          )}
        </span>
      </div>
    </article>
  );
}

interface BoardViewProps {
  issues: Issue[];
  config: AppConfig;
  currentView: Map<string, RunView>;
  launcher: Launcher;
  picks: Record<string, string>;
  onPick: (issueId: string, workflow: string) => void;
  selected: Set<string>;
  onToggle: (issueId: string) => void;
  onOpenIssue: (issueId: string) => void;
  onOpenRun: (key: string) => void;
  onMapRepo: () => void;
}

export function BoardView(p: BoardViewProps) {
  const grouped = useMemo(() => groupByColumn(p.issues), [p.issues]);
  const columns = COLUMNS.filter((c) => !c.hideWhenEmpty || grouped.get(c.id)!.length > 0);

  return (
    <div className="board" role="list" aria-label="Board">
      {columns.map((col) => {
        const list = grouped.get(col.id)!;
        return (
          <section key={col.id} className="column" role="listitem" aria-label={`${col.label}, ${list.length} issues`}>
            <h2 className="column-head">
              <span className={`column-ring col-${col.tone}`} aria-hidden />
              <span>{col.label}</span>
              <span className="column-count num">{list.length}</span>
            </h2>
            {list.map((issue) => {
              const repo = resolveRepo(p.config, issue.team.id, issue.project?.id);
              return (
                <IssueCard
                  key={issue.id}
                  issue={issue}
                  repo={repo}
                  view={p.currentView.get(issue.id)}
                  selected={p.selected.has(issue.id)}
                  catalog={p.launcher.catalogFor(issue)}
                  workflow={p.launcher.workflowFor(issue, p.picks[issue.id])}
                  launching={p.launcher.pending.has(issue.id)}
                  onToggle={() => p.onToggle(issue.id)}
                  onOpen={() => p.onOpenIssue(issue.id)}
                  onOpenRun={p.onOpenRun}
                  onPickWorkflow={(w) => p.onPick(issue.id, w)}
                  onRun={() => void p.launcher.launch([issue], p.picks[issue.id])}
                  onMapRepo={p.onMapRepo}
                />
              );
            })}
            {list.length === 0 && <div className="column-empty">No issues</div>}
          </section>
        );
      })}
    </div>
  );
}

const SKELETON = [3, 4, 3, 2, 2];

export function BoardSkeleton() {
  return (
    <div className="board" aria-busy="true" aria-label="Loading issues">
      {SKELETON.map((n, i) => (
        <div key={i} className="column">
          <div className="skel-head" />
          {Array.from({ length: n }, (_, k) => (
            <div key={k} className="skel-card" style={{ height: k % 2 ? 96 : 112 }}>
              <i style={{ width: 60 }} />
              <i style={{ width: "85%" }} />
              <i style={{ width: "55%" }} className="faint" />
            </div>
          ))}
        </div>
      ))}
    </div>
  );
}

export function EmptyState({
  title,
  text,
  action,
}: {
  title: string;
  text: string;
  action?: { label: string; onClick: () => void };
}) {
  return (
    <div className="center-state">
      <div className="center-state-body">
        <div className="state-icon-empty" aria-hidden />
        <div className="center-state-title">{title}</div>
        <div className="center-state-text">{text}</div>
        {action && (
          <button type="button" className="btn" onClick={action.onClick}>
            {action.label}
          </button>
        )}
      </div>
    </div>
  );
}

const ERROR_TITLES: Record<LinearError["kind"], string> = {
  missingKey: "No Linear API key",
  invalidKey: "Linear rejected your API key",
  network: "Couldn't reach Linear",
  rateLimited: "Linear is rate limiting requests",
  keychain: "Couldn't read the macOS Keychain",
  api: "Linear returned an error",
  unknown: "Something went wrong",
};

export function ErrorState({
  error,
  lastSync,
  onRetry,
  onSettings,
  retrying,
}: {
  error: LinearError;
  lastSync?: number | null;
  onRetry: () => void;
  onSettings?: () => void;
  retrying?: boolean;
}) {
  const keyProblem = error.kind === "invalidKey" || error.kind === "missingKey";
  return (
    <div className="center-state" role="alert">
      <div className="center-state-body">
        <div className="state-icon-error" aria-hidden>
          !
        </div>
        <div className="center-state-title">{ERROR_TITLES[error.kind]}</div>
        <div className="center-state-text">
          {error.message}
          {error.kind !== "keychain" && "\nRunning agents are unaffected."}
        </div>
        {error.kind !== "keychain" && (
          <div className="center-state-meta">
            api.linear.app{lastSync ? ` · last success ${formatClock(lastSync)}` : ""}
          </div>
        )}
        <div className="center-state-actions">
          <button type="button" className={keyProblem ? "btn" : "btn btn-primary"} onClick={onRetry} disabled={retrying}>
            {retrying ? "Retrying…" : "Retry"}
          </button>
          {onSettings && (
            <button type="button" className={keyProblem ? "btn btn-primary" : "btn"} onClick={onSettings}>
              Check API key
            </button>
          )}
        </div>
      </div>
    </div>
  );
}
