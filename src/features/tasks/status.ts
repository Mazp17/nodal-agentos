import type { Finish, Isolation, Priority, TaskStatus } from "../../domain/types";

export interface StatusMeta {
  label: string;
  /** CSS variable for the status color. */
  color: string;
  /** Dashed ring (in progress). */
  dashed: boolean;
}

export const STATUS_META: Record<TaskStatus, StatusMeta> = {
  backlog: { label: "Backlog", color: "var(--text-disabled)", dashed: true },
  todo: { label: "Todo", color: "var(--gray)", dashed: false },
  in_progress: { label: "In Progress", color: "var(--accent)", dashed: true },
  in_review: { label: "In Review", color: "var(--amber)", dashed: false },
  blocked: { label: "Blocked", color: "var(--red)", dashed: false },
  done: { label: "Done", color: "var(--green)", dashed: false },
  canceled: { label: "Canceled", color: "var(--text-disabled)", dashed: false },
};

/** Columns visible by default; Backlog and Canceled sit behind a filter. */
export const BOARD_COLUMNS: TaskStatus[] = ["todo", "in_progress", "in_review", "blocked", "done"];
export const HIDDEN_COLUMNS: TaskStatus[] = ["backlog", "canceled"];

export const isClosed = (s: TaskStatus) => s === "done" || s === "canceled";

export const PRIORITY_LABEL: Record<Priority, string> = {
  urgent: "Urgent",
  high: "High",
  medium: "Medium",
  low: "Low",
  none: "No priority",
};
export const PRIORITIES: Priority[] = ["urgent", "high", "medium", "low", "none"];

export const FINISH_LABEL: Record<Finish, string> = { changes: "Changes only", commit: "Commit", pr: "Open PR" };
export const FINISH_HINT: Record<Finish, string> = {
  changes: "Leaves the changes uncommitted",
  commit: "Commits on the task branch, no push",
  pr: "Commits, pushes and opens a PR",
};
export const ISOLATION_LABEL: Record<Isolation, string> = { worktree: "Worktree", in_place: "In place" };
export const ISOLATION_HINT: Record<Isolation, string> = {
  worktree: "Own branch and worktree folder",
  in_place: "Works in the repo folder; one in-place run per repo at a time",
};

export const PROVIDER_LABEL: Record<string, string> = { linear: "Linear" };
export const providerLabel = (p: string) => PROVIDER_LABEL[p] ?? p.charAt(0).toUpperCase() + p.slice(1);
