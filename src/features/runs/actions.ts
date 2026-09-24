import { useCallback, useState } from "react";
import { cancelRun, confirmRun, launchTask, reorderQueue } from "../../domain/api";
import { refreshRuns } from "../../domain/hooks/runs";
import type { Task } from "../../domain/types";
import { useToast } from "../../ui/Toasts";
import { attachRun } from "./api";
import { launchErrorHint } from "./LaunchBlockerNotice";
import type { RunView } from "./status";

export interface RunActions {
  /** Hay una acción en vuelo (para deshabilitar botones). */
  busy: boolean;
  /** Detiene un run lanzado (pide confirmación). */
  stop: (v: RunView, name: string) => Promise<boolean>;
  /** Saca de la cola un run `queued`. */
  remove: (v: RunView, name: string) => Promise<boolean>;
  confirm: (v: RunView, name: string) => Promise<boolean>;
  attach: (v: RunView) => Promise<void>;
  /** Encola otro run de la tarea con el mismo ejecutor. */
  runAgain: (v: RunView, task: Task | undefined, name: string) => Promise<boolean>;
  /** Nuevo orden de la cola global: ids de todos los runs `queued`. */
  reorder: (ids: string[]) => Promise<void>;
}

export function useRunActions(): RunActions {
  const toast = useToast();
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
    async (v: RunView, name: string) => {
      if (!window.confirm(`Stop ${name}? The agent is interrupted mid-task; its unfinished changes are saved as a patch.`)) {
        return false;
      }
      const ok = await wrap(() => cancelRun(v.run.id), `Couldn't stop ${name}`);
      if (ok) toast(`${name} stopped`, "The task moves to Blocked. The worktree is kept so you can inspect it.", "danger");
      return ok;
    },
    [wrap, toast],
  );

  const remove = useCallback(
    async (v: RunView, name: string) => {
      const ok = await wrap(() => cancelRun(v.run.id), `Couldn't remove ${name} from the queue`);
      if (ok) toast(`${name} removed from queue`, "It will not run.", "muted");
      return ok;
    },
    [wrap, toast],
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
