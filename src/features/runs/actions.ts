import { useCallback, useState } from "react";
import { resolveRepo, type AppConfig, type Issue } from "../linear/api";
import { useToast } from "../../ui/Toasts";
import { attachRun, cancelQueued, launchIssueRun, stopRun } from "./api";
import { launchErrorHint } from "./LaunchBlockerNotice";
import type { RunView } from "./status";
import type { RunsState } from "./useRuns";
import { pickWorkflow, type WorkflowCatalogs } from "./useWorkflows";
import type { WorkflowInfo } from "./types";

/** Un workflow para todas, o uno por issue (p. ej. el elegido en cada card). */
export type WorkflowChoice = string | null | ((issue: Issue) => string | null | undefined);

export interface Launcher {
  /** Issues con un lanzamiento en vuelo (para deshabilitar botones). */
  pending: Set<string>;
  catalogFor: (issue: Issue) => WorkflowInfo[];
  workflowFor: (issue: Issue, wanted?: string | null) => string;
  /** Lanza (o encola) cada issue con su workflow; avisa con toasts. */
  launch: (issues: Issue[], workflow?: WorkflowChoice) => Promise<void>;
}

export function useLauncher(
  config: AppConfig,
  catalogs: WorkflowCatalogs,
  lastWorkflow: string,
  runs: RunsState,
): Launcher {
  const toast = useToast();
  const [pending, setPending] = useState<Set<string>>(new Set());

  const catalogFor = useCallback(
    (issue: Issue) => {
      const repo = resolveRepo(config, issue.team.id, issue.project?.id);
      return (repo && catalogs[repo]) || catalogs[""] || [];
    },
    [config, catalogs],
  );
  const workflowFor = useCallback(
    (issue: Issue, wanted?: string | null) => pickWorkflow(catalogFor(issue), wanted ?? lastWorkflow),
    [catalogFor, lastWorkflow],
  );

  const launch = useCallback(
    async (issues: Issue[], workflow?: WorkflowChoice) => {
      const ids = issues.map((i) => i.id);
      setPending((p) => new Set([...p, ...ids]));
      const started: string[] = [];
      const queued: string[] = [];
      try {
        // En serie: el backend decide slot o cola en orden de llegada.
        for (const issue of issues) {
          try {
            const ir = await launchIssueRun({
              issueId: issue.id,
              identifier: issue.identifier,
              teamId: issue.team.id,
              projectId: issue.project?.id ?? null,
              workflow: workflowFor(issue, typeof workflow === "function" ? workflow(issue) : workflow),
            });
            if (ir.status === "failed") {
              toast(`Couldn't launch ${issue.identifier}`, launchErrorHint(ir.error, ir.cwd), "danger");
              continue;
            }
            (ir.status === "queued" ? queued : started).push(issue.identifier);
          } catch (e) {
            toast(`Couldn't launch ${issue.identifier}`, String(e), "danger");
          }
        }
      } finally {
        setPending((p) => new Set([...p].filter((id) => !ids.includes(id))));
        void runs.refresh();
      }
      if (started.length === 1) toast(`${started[0]} launched`, "Starting the workflow.", "ok");
      else if (started.length > 1) toast(`Launched ${started.length} runs`, started.join(", "), "ok");
      if (queued.length) {
        toast(
          queued.length === 1 ? `${queued[0]} queued` : `${queued.length} queued`,
          `${queued.length > 1 ? queued.join(", ") + " start" : "Starts"} when a slot frees up (limit ${config.concurrency}).`,
          "muted",
        );
      }
    },
    [workflowFor, toast, runs, config.concurrency],
  );

  return { pending, catalogFor, workflowFor, launch };
}

export interface RunActions {
  busy: boolean;
  stop: (v: RunView) => Promise<void>;
  attach: (v: RunView) => Promise<void>;
  cancel: (v: RunView) => Promise<boolean>;
}

export function useRunActions(runs: RunsState): RunActions {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const name = (v: RunView) => v.identifier ?? v.run?.name ?? v.runId ?? "Run";

  const wrap = useCallback(
    async (fn: () => Promise<void>, fail: string, refresh = true): Promise<boolean> => {
      setBusy(true);
      try {
        await fn();
        if (refresh) await runs.refresh();
        return true;
      } catch (e) {
        toast(fail, String(e), "danger");
        return false;
      } finally {
        setBusy(false);
      }
    },
    [runs, toast],
  );

  const stop = useCallback(
    async (v: RunView) => {
      if (!v.runId) return;
      if (!window.confirm(`Stop ${name(v)}? The agent is interrupted mid-task.`)) return;
      const ok = await wrap(() => stopRun(v.runId!), `Couldn't stop ${name(v)}`);
      if (ok) toast(`${name(v)} stopped`, `claude stop ${v.runId}`, "danger");
    },
    [wrap, toast],
  );

  const attach = useCallback(
    async (v: RunView) => {
      if (!v.runId) return;
      const ok = await wrap(() => attachRun(v.runId!), `Couldn't attach to ${name(v)}`, false);
      if (ok) toast("Attached in Terminal", `claude attach ${v.runId}`);
    },
    [wrap, toast],
  );

  const cancel = useCallback(
    async (v: RunView) => {
      if (!v.issueId) return false;
      // Sin esperar el refresh: quien llama puede navegar antes de que el run desaparezca.
      const ok = await wrap(() => cancelQueued(v.issueId!), `Couldn't remove ${name(v)} from the queue`, false);
      if (ok) {
        toast(`${name(v)} removed from queue`, "It will not run.", "muted");
        void runs.refresh();
      }
      return ok;
    },
    [wrap, toast, runs],
  );

  return { busy, stop, attach, cancel };
}
