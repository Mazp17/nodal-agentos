import { useCallback } from "react";
import { handOff, launchTask, reviewNow, type LaunchInput } from "../../domain/api";
import { invalidate } from "../../domain/hooks/store";
import type { Executor, Run } from "../../domain/types";
import { useToast } from "../../ui/Toasts";
import { executorLabel } from "../executors";

/** Toast depending on how the just-queued run ended up. */
function report(push: ReturnType<typeof useToast>, what: string, name: string, run: Run) {
  if (run.status === "failed") {
    push(`${what} failed`, run.error ?? "The run couldn't be started.", "danger");
  } else if (run.status === "queued") {
    push(`${name} queued`, "It starts when a slot frees up.", "warn");
  } else {
    push(`${name} launched`, `${executorLabel(run.executor)} is on it.`, "accent");
  }
}

export type LaunchKind = { kind: "run"; input?: LaunchInput } | { kind: "handoff"; executor: Executor; extra: string | null } | { kind: "review" };

/**
 * Launches, hands off or reviews a task with toasts and invalidation. Returns the run, or `null`
 * if the backend refused (the reason goes to the toast).
 */
export function useLaunch() {
  const push = useToast();
  return useCallback(
    async (taskId: string, name: string, how: LaunchKind): Promise<Run | null> => {
      try {
        const run =
          how.kind === "run"
            ? await launchTask(taskId, how.input ?? {})
            : how.kind === "handoff"
              ? await handOff(taskId, how.executor, how.extra)
              : await reviewNow(taskId, null);
        const what = how.kind === "review" ? "Review" : how.kind === "handoff" ? "Hand-off" : "Launch";
        report(push, what, name, run);
        return run;
      } catch (e) {
        push(`Couldn't start ${name}`, String(e), "danger");
        return null;
      } finally {
        invalidate("runs", "tasks");
      }
    },
    [push],
  );
}
