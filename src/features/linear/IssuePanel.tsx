import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Issue, IssueDetail, IssueRef, RelationKind, StateRef } from "./api";
import { useIssueDetail, type IssueDetailCache } from "./issueDetail";
import { PRIORITY_LABELS } from "./columns";
import type { RunActions } from "../runs/actions";
import { LaunchBlockerNotice } from "../runs/LaunchBlockerNotice";
import type { RunView } from "../runs/status";
import { isInProgress, type WorkflowInfo } from "../runs/types";
import { formatDuration, formatTokens } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { SafeMarkdown } from "../../ui/Markdown";
import "./issue-panel.css";

interface Props {
  issue: Issue;
  repo: string | null;
  history: RunView[];
  current: RunView | undefined;
  catalog: WorkflowInfo[];
  defaultWorkflow: string;
  launching: boolean;
  /** Slots libres ahora mismo: si no hay, el run queda en cola. */
  freeSlots: number;
  actions: RunActions;
  onLaunch: (workflow: string) => void;
  onOpenRun: (key: string) => void;
  onMapRepo: () => void;
  onViewQueue: () => void;
  detailCache: IssueDetailCache;
  /** Si la issue (por id) está cargada en el board y se puede abrir en el panel. */
  isOnBoard: (id: string) => boolean;
  onOpenIssue: (id: string) => void;
  onClose: () => void;
}

export function IssuePanel(p: Props) {
  const { issue, current } = p;
  const ref = useFocusTrap<HTMLDivElement>(p.onClose);
  const [detailState, retryDetail] = useIssueDetail(issue.id, p.detailCache);
  const detail = detailState.status === "ready" ? detailState.detail : null;
  const [picked, setPicked] = useState(p.defaultWorkflow);
  const who = issue.assignee?.displayName || issue.assignee?.name;
  const queued = current?.kind === "queued";
  const active = current?.active && !queued;
  const showLaunch = p.repo !== null && !current?.active;
  // Si el catálogo cargó después y no trae lo elegido, vale el default (que sí está).
  const workflow = !p.catalog.length || p.catalog.some((w) => w.name === picked) ? picked : p.defaultWorkflow;
  const setWorkflow = setPicked;
  const options = p.catalog.length ? p.catalog : [{ name: workflow, description: null, whenToUse: null }];

  const meta: [string, string][] = [
    ["Team", issue.team.name],
    ["Project", issue.project?.name ?? "None"],
    ["Priority", PRIORITY_LABELS[issue.priority] ?? issue.priorityLabel],
    ["Assignee", who ?? "Unassigned"],
  ];
  if (detail?.estimate != null) meta.push(["Estimate", String(detail.estimate)]);
  if (detail?.dueDate) meta.push(["Due", formatDay(detail.dueDate)]);
  if (detail) {
    const by = detail.creator ? ` by ${detail.creator.displayName || detail.creator.name}` : "";
    meta.push(["Created", `${formatDay(detail.createdAt)}${by}`]);
  }
  const openBlockers = detail?.relations.filter((r) => r.kind === "blocked_by" && !isFinished(r.issue.state)) ?? [];

  return (
    <>
      <div className="scrim" onClick={p.onClose} aria-hidden />
      <div
        ref={ref}
        className="sheet issue-panel"
        role="dialog"
        aria-modal="true"
        aria-labelledby="issue-panel-title"
        tabIndex={-1}
      >
        <header className="ip-head">
          <div className="ip-row">
            <span className="ip-id">{issue.identifier}</span>
            <span className="ip-state">
              <span className="dot-ring-state" style={{ color: issue.state.color }} aria-hidden />
              {issue.state.name}
            </span>
            <a
              href={issue.url}
              onClick={(e) => {
                e.preventDefault();
                openUrl(issue.url).catch((err) => console.error("openUrl", err));
              }}
            >
              Open in Linear ↗
            </a>
            <button type="button" className="icon-btn ip-close" aria-label="Close" onClick={p.onClose}>
              ✕
            </button>
          </div>
          <h2 id="issue-panel-title" className="ip-title">
            {issue.title}
          </h2>
          <div className="ip-meta">
            {meta.map(([k, v]) => (
              <span key={k}>
                {k} <span className="ip-meta-v">{v}</span>
              </span>
            ))}
            <span>
              Repo <span className="ip-meta-v mono">{p.repo ?? "None"}</span>
            </span>
          </div>
          {detail && detail.labels.length > 0 && (
            <ul className="ip-labels" aria-label="Labels">
              {detail.labels.map((l) => (
                <li key={l.id} className="ip-label">
                  <span className="dot" style={{ color: l.color }} aria-hidden />
                  {l.name}
                </li>
              ))}
            </ul>
          )}
        </header>

        <div className="ip-body">
          {openBlockers.length > 0 && (
            <div className="ip-blocked" role="status">
              <span className="dot" aria-hidden />
              <span>
                Blocked by {openBlockers.length} unfinished issue{openBlockers.length === 1 ? "" : "s"}:{" "}
                {openBlockers.map((r) => r.issue.identifier).join(", ")}
              </span>
            </div>
          )}

          {detailState.status === "loading" && (
            <div className="ip-loading" aria-busy="true" aria-label="Loading issue details">
              <span className="ip-skel" style={{ width: "92%" }} />
              <span className="ip-skel" style={{ width: "78%" }} />
              <span className="ip-skel" style={{ width: "55%" }} />
            </div>
          )}
          {detailState.status === "error" && (
            <div className="ip-error" role="alert">
              <span>Could not load issue details. {detailState.error.message}</span>
              <button type="button" className="btn btn-sm" onClick={retryDetail}>
                Retry
              </button>
            </div>
          )}
          {detail && <DetailSections detail={detail} isOnBoard={p.isOnBoard} onOpenIssue={p.onOpenIssue} />}

          <LaunchBlockerNotice view={current} compact />

          <section className="ip-section">
            <h3 className="section-label">Run history</h3>
            {p.history.map((v) => (
              <button key={v.key} type="button" className="ip-hist" onClick={() => p.onOpenRun(v.key)}>
                <span className="ip-hist-id">{v.runId ?? v.workflow}</span>
                <span className={`badge badge-sm tone-${v.tone}`}>{v.label}</span>
                <span className="ip-hist-wf ellipsis">{v.workflow}</span>
                <span className="ip-hist-num num">{v.kind === "queued" ? "—" : formatDuration(v.durationMs)}</span>
                <span className="ip-hist-num ip-hist-tok num">{formatTokens(v.tokens)}</span>
              </button>
            ))}
            {p.history.length === 0 && <span className="ip-empty">No runs yet.</span>}
          </section>

          {showLaunch && (
            <section className="ip-launch" aria-labelledby="ip-launch-label">
              <h3 id="ip-launch-label" className="section-label">
                Launch
              </h3>
              <div className="ip-wfs" role="radiogroup" aria-label="Workflow">
                {options.map((w) => (
                  <button
                    key={w.name}
                    type="button"
                    role="radio"
                    aria-checked={workflow === w.name}
                    className={`ip-wf ${workflow === w.name ? "on" : ""}`}
                    onClick={() => setWorkflow(w.name)}
                  >
                    <span className="ip-wf-name">{w.name}</span>
                    {(w.description || w.whenToUse) && <span className="ip-wf-desc">{w.description ?? w.whenToUse}</span>}
                  </button>
                ))}
              </div>
            </section>
          )}

          {detail && <Comments detail={detail} url={issue.url} />}
        </div>

        <footer className="ip-foot">
          {p.repo === null ? (
            <button type="button" className="btn btn-primary" onClick={p.onMapRepo}>
              Map a repo
            </button>
          ) : active && current ? (
            <>
              {isInProgress(current.run) && current.runId && (
                <button type="button" className="btn btn-danger" disabled={p.actions.busy} onClick={() => void p.actions.stop(current)}>
                  Stop
                </button>
              )}
              {current.runId && (
                <button type="button" className="btn" disabled={p.actions.busy} onClick={() => void p.actions.attach(current)}>
                  Attach
                </button>
              )}
              <button type="button" className="btn btn-primary" onClick={() => p.onOpenRun(current.key)}>
                Open run
              </button>
            </>
          ) : queued && current ? (
            <>
              {current.ir?.status === "queued" && (
                <button type="button" className="btn" disabled={p.actions.busy} onClick={() => void p.actions.cancel(current)}>
                  Remove from queue
                </button>
              )}
              <button type="button" className="btn btn-primary" onClick={p.onViewQueue}>
                View queue
              </button>
            </>
          ) : (
            <>
              {current && (
                <button type="button" className="btn" onClick={() => p.onOpenRun(current.key)}>
                  Open last run
                </button>
              )}
              <button
                type="button"
                className="btn btn-primary"
                disabled={p.launching}
                onClick={() => p.onLaunch(workflow)}
              >
                {p.launching ? "Launching…" : p.freeSlots > 0 ? "Run" : "Run (will queue)"}
              </button>
            </>
          )}
        </footer>
      </div>
    </>
  );
}

const FINISHED = new Set(["completed", "canceled"]);
function isFinished(state: StateRef): boolean {
  return FINISHED.has(state.type);
}

/** "2026-10-01" o ISO completo → "Oct 1, 2026". Las fechas sin hora se leen como locales. */
function formatDay(value: string): string {
  const d = /^\d{4}-\d{2}-\d{2}$/.test(value) ? new Date(`${value}T00:00:00`) : new Date(value);
  return Number.isNaN(d.getTime()) ? value : d.toLocaleDateString([], { dateStyle: "medium" });
}

function formatWhen(iso: string): string {
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
}

const RELATION_GROUPS: { kind: RelationKind; label: string; tone: string }[] = [
  { kind: "blocked_by", label: "Blocked by", tone: "danger" },
  { kind: "blocks", label: "Blocks", tone: "warn" },
  { kind: "related", label: "Related", tone: "muted" },
  { kind: "duplicate", label: "Duplicate", tone: "muted" },
];

interface NavProps {
  isOnBoard: (id: string) => boolean;
  onOpenIssue: (id: string) => void;
}

/** Fila de issue relacionada: botón si está en el board (abre su detalle), texto si no. */
function IssueRow({ issue, note, warn, nav }: { issue: IssueRef; note?: string; warn?: boolean; nav: NavProps }) {
  const inner = (
    <>
      <span className="dot-ring-state" style={{ color: issue.state.color }} aria-hidden />
      <span className="ip-rel-id">{issue.identifier}</span>
      <span className="ip-rel-title ellipsis">{issue.title}</span>
      {warn && <span className="badge badge-sm tone-danger">Not done</span>}
      <span className="ip-rel-state">{note ?? issue.state.name}</span>
    </>
  );
  return nav.isOnBoard(issue.id) ? (
    <button type="button" className="ip-rel ip-rel-link" onClick={() => nav.onOpenIssue(issue.id)}>
      {inner}
    </button>
  ) : (
    <div className="ip-rel" title="Not on the board">
      {inner}
    </div>
  );
}

function DetailSections({ detail, ...nav }: { detail: IssueDetail } & NavProps) {
  const done = detail.children.filter((c) => c.state.type === "completed").length;
  const groups = RELATION_GROUPS.map((g) => ({ ...g, items: detail.relations.filter((r) => r.kind === g.kind) })).filter(
    (g) => g.items.length > 0,
  );
  return (
    <>
      <section className="ip-section" aria-label="Description">
        {detail.description ? (
          <SafeMarkdown text={detail.description} />
        ) : (
          <span className="ip-empty">No description.</span>
        )}
      </section>

      {detail.parent && (
        <section className="ip-section">
          <h3 className="section-label">Parent</h3>
          <IssueRow issue={detail.parent} nav={nav} />
        </section>
      )}

      {detail.children.length > 0 && (
        <section className="ip-section">
          <h3 className="section-label">
            Sub-issues{" "}
            <span className="ip-count num">
              {done}/{detail.children.length}
              {detail.childrenTruncated ? "+" : ""}
            </span>
          </h3>
          {detail.children.map((c) => {
            const who = c.assignee ? c.assignee.displayName || c.assignee.name : null;
            return <IssueRow key={c.id} issue={c} note={who ? `${c.state.name} · ${who}` : undefined} nav={nav} />;
          })}
          {detail.childrenTruncated && <span className="ip-empty">More sub-issues in Linear.</span>}
        </section>
      )}

      {groups.length > 0 && (
        <section className="ip-section">
          <h3 className="section-label">Relations</h3>
          {groups.map((g) => (
            <div key={g.kind} className="ip-rel-group">
              <span className={`ip-rel-kind tone-${g.tone}`}>{g.label}</span>
              <div className="ip-rel-list">
                {g.items.map((r) => (
                  <IssueRow
                    key={`${r.kind}-${r.inverse}-${r.issue.id}`}
                    issue={r.issue}
                    warn={r.kind === "blocked_by" && !isFinished(r.issue.state)}
                    note={r.kind === "duplicate" ? (r.inverse ? "Duplicates this" : "Original") : undefined}
                    nav={nav}
                  />
                ))}
              </div>
            </div>
          ))}
        </section>
      )}
    </>
  );
}

function Comments({ detail, url }: { detail: IssueDetail; url: string }) {
  if (detail.comments.length === 0) return null;
  return (
    <section className="ip-section" aria-labelledby="ip-comments-label">
      <h3 id="ip-comments-label" className="section-label">
        Comments{" "}
        <span className="ip-count num">
          {detail.comments.length}
          {detail.commentsTruncated ? "+" : ""}
        </span>
      </h3>
      {detail.commentsTruncated && (
        <span className="ip-empty">
          Showing {detail.comments.length} comments.{" "}
          <a
            href={url}
            onClick={(e) => {
              e.preventDefault();
              openUrl(url).catch((err) => console.error("openUrl", err));
            }}
          >
            See all in Linear ↗
          </a>
        </span>
      )}
      {detail.comments.map((c) => (
        <article key={c.id} className="ip-comment">
          <header className="ip-comment-head">
            <span className="ip-comment-author">{c.author ?? "Unknown"}</span>
            <time dateTime={c.createdAt}>{formatWhen(c.createdAt)}</time>
          </header>
          <SafeMarkdown text={c.body} />
        </article>
      ))}
    </section>
  );
}
