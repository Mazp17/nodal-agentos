import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { Issue } from "./api";
import { PRIORITY_LABELS } from "./columns";
import type { RunActions } from "../runs/actions";
import type { RunView } from "../runs/status";
import type { WorkflowInfo } from "../runs/types";
import { formatDuration, formatTokens } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
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
  onClose: () => void;
}

export function IssuePanel(p: Props) {
  const { issue, current } = p;
  const ref = useFocusTrap<HTMLDivElement>(p.onClose);
  const [workflow, setWorkflow] = useState(p.defaultWorkflow);
  const who = issue.assignee?.displayName || issue.assignee?.name;
  const queued = current?.kind === "queued";
  const active = current?.active && !queued;
  const showLaunch = p.repo !== null && !current?.active;
  const options = p.catalog.length ? p.catalog : [{ name: workflow, description: null, whenToUse: null }];

  const meta: [string, string][] = [
    ["Team", issue.team.name],
    ["Project", issue.project?.name ?? "None"],
    ["Priority", PRIORITY_LABELS[issue.priority] ?? issue.priorityLabel],
    ["Assignee", who ?? "Unassigned"],
  ];

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
        </header>

        <div className="ip-body">
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
        </div>

        <footer className="ip-foot">
          {p.repo === null ? (
            <button type="button" className="btn btn-primary" onClick={p.onMapRepo}>
              Map a repo
            </button>
          ) : active && current ? (
            <>
              {current.run?.state === "working" && current.runId && (
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
