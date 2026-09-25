import { useEffect, useState } from "react";
import { useToast } from "../../ui/Toasts";
import { getLaunchBlocker, openTerminalAt } from "./api";
import type { RunView } from "./status";
import "./launch-blocker.css";

/**
 * Known reasons Claude Code won't let a run start, each with a clear action.
 *
 * - `trust`: `claude --bg` refuses in a repo that was never opened with `claude`
 *   ("Workspace not trusted. Run `claude` in <dir> once and accept the trust prompt…").
 *   It comes out synchronously on stderr, so it arrives in the launch error (`launchError`).
 * - `workflowReview`: the workflow is new or changed and nobody approved it in `/workflows`.
 *   The `--bg` session does start: the rejection lands as a `tool_result` in its transcript
 *   and the session ends without a workflow ("Interrupted"). Only visible by reading the
 *   transcript (`get_launch_blocker`); `claude agents` doesn't report it.
 */
export type Blocker =
  | { kind: "trust"; dir: string | null; home: boolean }
  | { kind: "workflowReview"; workflow: string | null };

const TRUST_RE = /Workspace not trusted/i;
const REVIEW_TEXT = "Review dynamic workflow before running";

/** Detects a known reason in a launch's error text. */
export function classifyLaunchError(text: string | null | undefined, fallbackDir: string | null): Blocker | null {
  if (!text) return null;
  if (TRUST_RE.test(text)) {
    // "Run `claude` in <dir> once …" / "Please run `claude` in <dir> first …"
    const m = /run `claude` in (.+?) (?:once|first)\b/i.exec(text);
    return { kind: "trust", dir: m?.[1]?.trim() || fallbackDir, home: /home directory/i.test(text) };
  }
  if (text.includes(REVIEW_TEXT)) return { kind: "workflowReview", workflow: null };
  return null;
}

/** Text for a failed-launch toast: the actionable reason if recognized. */
export function launchErrorHint(text: string | null | undefined, dir: string | null): string | undefined {
  const b = classifyLaunchError(text, dir);
  if (b?.kind === "trust" && !b.home) {
    return `Workspace not trusted. Open Claude once in ${b.dir ?? "the repository"} and accept the trust dialog (the run has a button for it).`;
  }
  return text ?? undefined;
}

// A finished session doesn't change: the `get_launch_blocker` result is cached.
const blockerCache = new Map<string, Blocker | null>();

/** Workflow run that ended without a result: likely stopped by the approval check. */
function endedWithoutResult(v: RunView): boolean {
  if (v.run.executor.kind !== "workflow" || !v.run.sessionId) return false;
  if (v.phase === "failed") return true;
  if (v.phase !== "finished") return false;
  return v.run.outcome == null || v.run.outcome === "unknown" || v.detail === null;
}

const workflowOf = (v: RunView) => (v.run.executor.kind === "workflow" ? v.run.executor.name : null);

export function useLaunchBlocker(view: RunView | undefined): Blocker | null {
  const fromError = view ? classifyLaunchError(view.run.error, view.run.cwd) : null;
  const sid = view && !fromError && endedWithoutResult(view) ? view.run.sessionId : null;
  const cwd = view?.run.cwd ?? "";
  const wf = view ? workflowOf(view) : null;
  const [fetched, setFetched] = useState<{ sid: string; blocker: Blocker | null } | null>(null);

  useEffect(() => {
    if (!sid) return;
    if (blockerCache.has(sid)) {
      setFetched({ sid, blocker: blockerCache.get(sid) ?? null });
      return;
    }
    let alive = true;
    getLaunchBlocker(sid, cwd)
      .then((b) => {
        const blocker: Blocker | null = b ? { kind: "workflowReview", workflow: b.workflow ?? wf } : null;
        blockerCache.set(sid, blocker);
        if (alive) setFetched({ sid, blocker });
      })
      .catch((e) => console.error("get_launch_blocker", e));
    return () => {
      alive = false;
    };
  }, [sid, cwd, wf]);

  if (fromError) return fromError.kind === "workflowReview" ? { ...fromError, workflow: wf } : fromError;
  return sid && fetched?.sid === sid ? fetched.blocker : null;
}

const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/** Actionable notice for a run blocked by workspace trust or workflow approval. */
export function LaunchBlockerNotice({
  view,
  compact,
  blocker: given,
}: {
  view: RunView | undefined;
  compact?: boolean;
  /** If already computed with `useLaunchBlocker`, to avoid repeating the query. */
  blocker?: Blocker | null;
}) {
  const own = useLaunchBlocker(given === undefined ? view : undefined);
  const blocker = given === undefined ? own : given;
  const toast = useToast();
  const [opening, setOpening] = useState(false);
  if (!blocker) return null;

  if (blocker.kind === "workflowReview") {
    const wf = blocker.workflow ?? "the workflow";
    return (
      <div className={`lb ${compact ? "lb-compact" : ""}`} role="alert">
        <span className="dot dot-lg" aria-hidden />
        <div className="lb-body">
          <span className="lb-title">Workflow not approved</span>
          <span className="lb-text">
            Claude Code asks to review new or changed workflows before running them, and a background session can't
            ask. Approve <span className="mono">{wf}</span> in <span className="mono">/workflows</span> in any Claude
            session, then run again.
          </span>
        </div>
      </div>
    );
  }

  const dir = blocker.dir;
  const open = async () => {
    if (!dir) return;
    setOpening(true);
    try {
      await openTerminalAt(dir, true);
      toast("Opened Terminal", `Accept the trust dialog in ${basename(dir)}, then run again.`);
    } catch (e) {
      toast("Couldn't open Terminal", String(e), "danger");
    } finally {
      setOpening(false);
    }
  };
  return (
    <div className={`lb ${compact ? "lb-compact" : ""}`} role="alert">
      <span className="dot dot-lg" aria-hidden />
      <div className="lb-body">
        <span className="lb-title">Workspace not trusted</span>
        <span className="lb-text">
          {blocker.home ? (
            "Claude Code never saves trust for your home directory. Map a project folder instead."
          ) : (
            <>
              Open Claude once in <span className="mono">{dir ?? "the repository"}</span> and accept the trust dialog,
              then run again.
            </>
          )}
        </span>
      </div>
      {dir && !blocker.home && (
        <button type="button" className="btn btn-sm" disabled={opening} onClick={() => void open()}>
          {opening ? "Opening…" : "Open in Terminal"}
        </button>
      )}
    </div>
  );
}
