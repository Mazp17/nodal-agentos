import { taskKey, type Priority, type Task, type TaskStatus } from "../../domain/types";

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
  in_progress: { label: "In Progress", color: "var(--st-in-progress)", dashed: true },
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

export const PROVIDER_LABEL: Record<string, string> = { linear: "Linear" };
export const providerLabel = (p: string) => PROVIDER_LABEL[p] ?? p.charAt(0).toUpperCase() + p.slice(1);

/** The short id of a task: its provider identifier (`NOD-12`), else the local key. */
export const taskTag = (t: Task, projectKey: string | undefined) =>
  t.source?.identifier ?? (projectKey ? taskKey(projectKey, t.number) : `#${t.number}`);
