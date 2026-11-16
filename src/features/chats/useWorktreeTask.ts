import { useAllChats } from "../../domain/hooks/chats";
import { useTasks } from "../../domain/hooks/store";
import type { Repo } from "../../domain/types";

/** The task working in a worktree of `repo` and its chat (`undefined` for the repo's own checkout). */
export function useWorktreeTask(repo: Repo, worktree: string | null) {
  const task = useTasks(repo.projectId).data?.find((t) => worktree !== null && t.repoId === repo.id && t.worktree?.path.replace(/\/+$/, "") === worktree.replace(/\/+$/, ""));
  const chat = useAllChats(!!task).data?.find((c) => c.taskId === task?.id);
  return { task, chat };
}
