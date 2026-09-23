import type { Issue } from "./api";

/*
 * Decisión de columnas: se agrupa por `state.type` (el tipo semántico que Linear
 * garantiza igual en todos los teams), no por nombre de estado. Los nombres varían
 * entre teams ("Todo" / "To do" / "Ready") y agruparlos por nombre obliga a mantener
 * tablas de equivalencias; el tipo es estable. La única excepción es "review": Linear
 * no tiene un tipo para eso, así que los estados `started` cuyo nombre contiene
 * "review" van a su propia columna. Como una columna mezcla estados (p. ej. "Blocked"
 * cae en "En curso"), cada card muestra su estado real con el color de Linear.
 * Triage sólo aparece si tiene issues.
 */

export type ColumnId =
  | "triage"
  | "backlog"
  | "unstarted"
  | "started"
  | "review"
  | "completed"
  | "canceled";

/** `tone` pinta el anillo de la columna (clase `col-<tone>` en board.css). */
export const COLUMNS: { id: ColumnId; label: string; tone: string; hideWhenEmpty?: boolean }[] = [
  { id: "triage", label: "Triage", tone: "amber", hideWhenEmpty: true },
  { id: "backlog", label: "Backlog", tone: "gray" },
  { id: "unstarted", label: "Todo", tone: "light" },
  { id: "started", label: "In Progress", tone: "accent" },
  { id: "review", label: "In Review", tone: "amber" },
  { id: "completed", label: "Done", tone: "green" },
  { id: "canceled", label: "Canceled", tone: "dim" },
];

export function columnOf(issue: Issue): ColumnId {
  const t = issue.state.type;
  if (t === "started" && /review/i.test(issue.state.name)) return "review";
  if (COLUMNS.some((c) => c.id === t)) return t as ColumnId;
  return "backlog";
}

/** Urgente primero, "sin prioridad" (0) al final; desempata lo más reciente. */
function compareIssues(a: Issue, b: Issue): number {
  const pa = a.priority === 0 ? 5 : a.priority;
  const pb = b.priority === 0 ? 5 : b.priority;
  if (pa !== pb) return pa - pb;
  return b.updatedAt.localeCompare(a.updatedAt);
}

export function groupByColumn(issues: Issue[]): Map<ColumnId, Issue[]> {
  const out = new Map<ColumnId, Issue[]>(COLUMNS.map((c) => [c.id, []]));
  for (const issue of issues) out.get(columnOf(issue))!.push(issue);
  for (const list of out.values()) list.sort(compareIssues);
  return out;
}

export const PRIORITY_LABELS: Record<number, string> = {
  0: "No priority",
  1: "Urgent",
  2: "High",
  3: "Medium",
  4: "Low",
};
