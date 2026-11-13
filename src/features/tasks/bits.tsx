import type { Priority, TaskPr, TaskStatus } from "../../domain/types";
import { ExternalLink } from "../../ui/ExternalLink";
import { PRIORITY_LABEL, STATUS_META } from "./status";
import "./tasks.css";

/** Status ring (dashed while in progress). */
export function StatusRing({ status, size = 12 }: { status: TaskStatus; size?: number }) {
  const m = STATUS_META[status];
  return (
    <span
      className="st-ring"
      aria-hidden
      style={{ width: size, height: size, borderColor: m.color, borderStyle: m.dashed ? "dashed" : "solid" }}
    />
  );
}

/** Three priority bars, like Linear. Urgent in red. */
export function PriorityBars({ priority }: { priority: Priority }) {
  const on = { urgent: 3, high: 3, medium: 2, low: 1, none: 0 }[priority];
  return (
    <span className="prio" role="img" aria-label={`Priority: ${PRIORITY_LABEL[priority]}`} title={PRIORITY_LABEL[priority]}>
      {[5, 8, 11].map((h, k) => (
        <i key={h} style={{ height: h }} className={priority === "urgent" ? "urgent" : k < on ? "on" : ""} />
      ))}
    </span>
  );
}

export const PR_TONE = { open: "tone-ok", draft: "tone-muted", merged: "tone-merged", closed: "tone-danger" } as const;

/** `#123` chip coloured by PR state; `link` makes it open the PR in the browser. */
export function PrBadge({ pr, link = false }: { pr: TaskPr; link?: boolean }) {
  const chip = (
    <span className={`badge badge-sm mono ${PR_TONE[pr.state]}`} title={`PR #${pr.number} · ${pr.state}`}>
      #{pr.number}
    </span>
  );
  return link ? (
    <ExternalLink url={pr.url} label={`Open PR #${pr.number} (${pr.state})`}>
      {chip}
    </ExternalLink>
  ) : (
    chip
  );
}
