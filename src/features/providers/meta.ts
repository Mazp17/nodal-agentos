import type { ExtKind, TaskStatus } from "../../domain/types";
import type { MapOrigin } from "../../domain/api";

export interface ProviderInfo {
  id: string;
  name: string;
  /** Has a backend; the rest show as "Coming soon". */
  available: boolean;
  keyPlaceholder?: string;
  /** Where the key is created, for the help text. */
  keyHelp?: string;
}

export const PROVIDERS: readonly ProviderInfo[] = [
  {
    id: "linear",
    name: "Linear",
    available: true,
    keyPlaceholder: "lin_api_…",
    keyHelp: "Create a personal API key in Linear → Settings → Security & access.",
  },
  { id: "asana", name: "Asana", available: false },
  { id: "azure_devops", name: "Azure DevOps", available: false },
  { id: "github", name: "GitHub Issues", available: false },
];

export function providerName(id: string): string {
  return PROVIDERS.find((p) => p.id === id)?.name ?? id;
}

export const STATUS_LABEL: Record<TaskStatus, string> = {
  backlog: "Backlog",
  todo: "Todo",
  in_progress: "In Progress",
  in_review: "In Review",
  blocked: "Blocked",
  done: "Done",
  canceled: "Canceled",
};

/** Color of each Nodal state (token variables). */
export const STATUS_COLOR: Record<TaskStatus, string> = {
  backlog: "var(--text-disabled)",
  todo: "var(--text-dim)",
  in_progress: "var(--accent)",
  in_review: "var(--amber)",
  blocked: "var(--red)",
  done: "var(--green)",
  canceled: "var(--text-disabled)",
};

export const EXT_KIND_LABEL: Record<ExtKind, string> = {
  triage: "Triage",
  backlog: "Backlog",
  unstarted: "Unstarted",
  started: "Started",
  completed: "Completed",
  canceled: "Canceled",
  unknown: "Other",
};

export const ORIGIN_LABEL: Record<MapOrigin, string> = {
  suggested: "Suggested",
  confirmed: "Confirmed",
  unmapped: "Unmapped",
};

/** "12s ago", "3m ago", "2h ago", "4d ago". */
export function formatAgo(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.floor((now - ms) / 1000));
  if (s < 60) return "just now";
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

export function plural(n: number, one: string, many = `${one}s`): string {
  return `${n} ${n === 1 ? one : many}`;
}

/**
 * Name of the unmapped state, taken from `syncError` (`External state "X" is not mapped…`).
 * Whether it's unmapped is told by `TaskSource.unmapped`; this is only for showing the name.
 */
export function unmappedStateName(syncError: string): string | null {
  return /"([^"]+)"/.exec(syncError)?.[1] ?? null;
}
