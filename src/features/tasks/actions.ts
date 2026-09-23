import { useCallback, useState } from "react";
import { useToast } from "../../ui/Toasts";
import { attachRun, stopRun } from "../runs/api";
import { launchErrorHint } from "../runs/LaunchBlockerNotice";
import type { RunView } from "../runs/status";
import { cancelTaskRun, deleteTask, launchTaskRun, setTaskDone } from "./api";
import type { FinishMode, Task } from "./types";

export interface TaskActions {
  /** Ids de tareas con una acción en vuelo. */
  pending: Set<string>;
  /** Sin `finish`, el backend usa el del repo en Settings (o "pr"). */
  run: (task: Task, finish?: FinishMode | null) => Promise<void>;
  toggleDone: (task: Task) => Promise<void>;
  cancel: (task: Task) => Promise<void>;
  remove: (task: Task) => Promise<boolean>;
  stop: (task: Task, v: RunView) => Promise<void>;
  attach: (task: Task, v: RunView) => Promise<void>;
}

/** Acciones sobre tareas con toasts; `refresh` se llama después de cada cambio. */
export function useTaskActions(refresh: () => Promise<void>): TaskActions {
  const toast = useToast();
  const [pending, setPending] = useState<Set<string>>(new Set());

  const wrap = useCallback(
    async (task: Task, fn: () => Promise<void>, fail: string): Promise<boolean> => {
      setPending((p) => new Set(p).add(task.id));
      try {
        await fn();
        return true;
      } catch (e) {
        toast(fail, String(e), "danger");
        return false;
      } finally {
        setPending((p) => {
          const n = new Set(p);
          n.delete(task.id);
          return n;
        });
        void refresh();
      }
    },
    [toast, refresh],
  );

  const run = useCallback(
    async (task: Task, finish?: FinishMode | null) => {
      await wrap(
        task,
        async () => {
          const tr = await launchTaskRun(task.id, finish ?? null);
          if (tr.status === "queued") toast(`${task.title} queued`, "Starts when a slot frees up.", "muted");
          else if (tr.status === "failed") toast(`Couldn't launch ${task.title}`, launchErrorHint(tr.error, tr.cwd), "danger");
          else toast(`${task.title} launched`, "Running /plan-task.", "ok");
        },
        `Couldn't launch ${task.title}`,
      );
    },
    [wrap, toast],
  );

  const toggleDone = useCallback(
    async (task: Task) => {
      await wrap(task, () => setTaskDone(task.id, task.status !== "done").then(() => {}), `Couldn't update ${task.title}`);
    },
    [wrap],
  );

  const cancel = useCallback(
    async (task: Task) => {
      const ok = await wrap(task, () => cancelTaskRun(task.id), `Couldn't remove ${task.title} from the queue`);
      if (ok) toast(`${task.title} removed from queue`, "It will not run.", "muted");
    },
    [wrap, toast],
  );

  const remove = useCallback(
    async (task: Task) => {
      if (!window.confirm(`Delete "${task.title}"? Its run history is removed too. Running sessions are not stopped.`)) {
        return false;
      }
      return wrap(task, () => deleteTask(task.id), `Couldn't delete ${task.title}`);
    },
    [wrap],
  );

  const stop = useCallback(
    async (task: Task, v: RunView) => {
      if (!v.runId) return;
      if (!window.confirm(`Stop ${task.title}? The agent is interrupted mid-task.`)) return;
      const ok = await wrap(task, () => stopRun(v.runId!), `Couldn't stop ${task.title}`);
      if (ok) toast(`${task.title} stopped`, `claude stop ${v.runId}`, "danger");
    },
    [wrap, toast],
  );

  const attach = useCallback(
    async (task: Task, v: RunView) => {
      if (!v.runId) return;
      const ok = await wrap(task, () => attachRun(v.runId!), `Couldn't attach to ${task.title}`);
      if (ok) toast("Attached in Terminal", `claude attach ${v.runId}`);
    },
    [wrap, toast],
  );

  return { pending, run, toggleDone, cancel, remove, stop, attach };
}
