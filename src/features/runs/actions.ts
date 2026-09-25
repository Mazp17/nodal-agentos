import { useCallback, useState } from "react";
import { cancelRun, confirmRun, launchTask, reorderQueue } from "../../domain/api";
import { refreshRuns } from "../../domain/hooks/runs";
import type { Task } from "../../domain/types";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { attachRun } from "./api";
import { launchErrorHint } from "./LaunchBlockerNotice";
import type { RunView } from "./status";

export interface RunActions {
  /** An action is in flight (to disable buttons). */
  busy: boolean;
  /** Stops a launched run (asks for confirmation). Only uses `v.run`. */
  stop: (v: Pick<RunView, "run">, name: string) => Promise<boolean>;
  /** Removes a `queued` run from the queue (light confirmation). Only uses `v.run`. */
  remove: (v: Pick<RunView, "run">, name: string) => Promise<boolean>;
  confirm: (v: RunView, name: string) => Promise<boolean>;
  attach: (v: RunView) => Promise<void>;
  /** Queues another run of the task with the same executor. */
  runAgain: (v: RunView, task: Task | undefined, name: string) => Promise<boolean>;
  /** New global queue order: ids of every `queued` run. */
  reorder: (ids: string[]) => Promise<void>;
}

export function useRunActions(): RunActions {
  const toast = useToast();
  const ask = useConfirm();
  const [busy, setBusy] = useState(false);

  const wrap = useCallback(
    async (fn: () => Promise<unknown>, fail: string): Promise<boolean> => {
      setBusy(true);
      try {
        await fn();
        void refreshRuns();
        return true;
      } catch (e) {
        toast(fail, String(e), "danger");
        return false;
      } finally {
        setBusy(false);
      }
    },
    [toast],
  );

  const stop = useCallback(
    async (v: Pick<RunView, "run">, name: string) => {
      const ok = await ask({
        title: `Stop ${name}?`,
        body: "The agent is interrupted mid-task. Its unfinished changes are saved as a patch and the worktree is kept.",
        confirmLabel: "Stop run",
      });
      if (!ok) return false;
      const stopped = await wrap(() => cancelRun(v.run.id), `Couldn't stop ${name}`);
      if (stopped) toast(`${name} stopped`, "The task moves to Blocked. The worktree is kept so you can inspect it.", "danger");
      return stopped;
    },
    [wrap, toast, ask],
  );

  const remove = useCallback(
    async (v: Pick<RunView, "run">, name: string) => {
      // Light confirmation, not undo: the queue has no "re-queue in the same spot";
      // undoing would mean launching a new run at the end of the queue.
      const confirmed = await ask({
        title: `Remove ${name} from the queue?`,
        body: "It won't run and loses its place in the queue. The task itself is kept; you can launch it again.",
        confirmLabel: "Remove",
      });
      if (!confirmed) return false;
      const ok = await wrap(() => cancelRun(v.run.id), `Couldn't remove ${name} from the queue`);
      if (ok) toast(`${name} removed from queue`, "It will not run.", "muted");
      return ok;
    },
    [wrap, toast, ask],
  );

  const confirm = useCallback(
    async (v: RunView, name: string) => {
      const ok = await wrap(() => confirmRun(v.run.id), `Couldn't confirm ${name}`);
      if (ok) toast(`${name} confirmed`, "It is back in the queue and starts when a slot frees up.", "ok");
      return ok;
    },
    [wrap, toast],
  );

  const attach = useCallback(
    async (v: RunView) => {
      const id = v.run.claudeRunId;
      if (!id) return;
      const ok = await wrap(() => attachRun(id), "Couldn't attach to the session");
      if (ok) toast("Attached in Terminal", `claude attach ${id}`);
    },
    [wrap, toast],
  );

  const runAgain = useCallback(
    async (v: RunView, task: Task | undefined, name: string) => {
      const taskId = v.run.taskId;
      if (!taskId) return false;
      setBusy(true);
      try {
        const run = await launchTask(taskId, { executor: v.run.executor });
        void refreshRuns();
        if (run.status === "failed") {
          toast(`Couldn't launch ${name}`, launchErrorHint(run.error, run.cwd), "danger");
          return false;
        }
        toast(
          run.status === "queued" ? `${name} queued` : `${name} launched`,
          run.status === "queued" ? "It starts when a slot frees up." : task?.title,
          run.status === "queued" ? "muted" : "ok",
        );
        return true;
      } catch (e) {
        toast(`Couldn't launch ${name}`, String(e), "danger");
        return false;
      } finally {
        setBusy(false);
      }
    },
    [toast],
  );

  const reorder = useCallback(
    async (ids: string[]) => {
      await wrap(() => reorderQueue(ids), "Couldn't reorder the queue");
    },
    [wrap],
  );

  return { busy, stop, remove, confirm, attach, runAgain, reorder };
}
