import type { ExecutorInfo } from "../../domain/api";
import { isExecutorHidden } from "../../domain/hooks/store";
import type { Executor, Project, Repo, Task } from "../../domain/types";

export const CLAUDE: Executor = { kind: "claude" };

/** Stable key for comparisons and lists. */
export function executorKey(e: Executor): string {
  switch (e.kind) {
    case "agent":
      return `agent:${e.source}:${e.name}`;
    case "workflow":
      return `workflow:${e.name}`;
    case "claude":
      return "claude";
  }
}

export const sameExecutor = (a: Executor | null | undefined, b: Executor | null | undefined) =>
  !!a && !!b && executorKey(a) === executorKey(b);

export function executorLabel(e: Executor): string {
  return e.kind === "claude" ? "Claude" : e.name;
}

export function executorKindLabel(e: Executor): string {
  return e.kind === "agent" ? "Agent" : e.kind === "workflow" ? "Workflow" : "Claude session";
}

/** Inherited default (ignoring the task): repo → project → Settings (`global`) → Claude. */
export function inheritedExecutor(
  repo: Repo | null | undefined,
  project: Project | null | undefined,
  global: Executor | null | undefined = null,
): Executor {
  return repo?.defaultExecutor ?? project?.defaultExecutor ?? global ?? CLAUDE;
}

/** Effective assignee: task → repo → project → Settings (`global`) → Claude. */
export function resolveExecutor(
  task: Pick<Task, "assignee"> | null | undefined,
  repo: Repo | null | undefined,
  project: Project | null | undefined,
  global: Executor | null | undefined = null,
): Executor {
  return task?.assignee ?? inheritedExecutor(repo, project, global);
}

/**
 * The catalog minus what the project hides; `keep` (the current value) stays so a hidden
 * assignment still shows as selected instead of reading as a deleted executor.
 */
export function visibleExecutors(
  catalog: readonly ExecutorInfo[],
  hidden: ReadonlySet<string>,
  repoId: string | null,
  keep: Executor | null | undefined,
): ExecutorInfo[] {
  if (hidden.size === 0) return [...catalog];
  return catalog.filter((i) => sameExecutor(i.executor, keep) || !isExecutorHidden(hidden, i, repoId));
}
