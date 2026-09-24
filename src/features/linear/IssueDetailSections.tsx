import type { IssueDetail, IssueRef, RelationKind, StateRef } from "./api";
import { ExternalLink } from "../../ui/ExternalLink";
import { SafeMarkdown } from "../../ui/Markdown";
import "./issue-panel.css";

// Secciones del detalle de una issue (descripción, padre, sub-issues, relaciones y
// comentarios). Las usa la pestaña Source del detalle de tarea.

const FINISHED = new Set(["completed", "canceled"]);
export function isFinished(state: StateRef): boolean {
  return FINISHED.has(state.type);
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

export interface NavProps {
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
    <div className="ip-rel" title="Not imported into Nodal">
      {inner}
    </div>
  );
}

export function DetailSections({ detail, ...nav }: { detail: IssueDetail } & NavProps) {
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

export function Comments({ detail, url }: { detail: IssueDetail; url: string }) {
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
          <ExternalLink url={url}>See all in Linear ↗</ExternalLink>
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
