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

export const COLUMNS: { id: ColumnId; label: string; hideWhenEmpty?: boolean }[] = [
  { id: "triage", label: "Triage", hideWhenEmpty: true },
  { id: "backlog", label: "Backlog" },
  { id: "unstarted", label: "Por hacer" },
  { id: "started", label: "En curso" },
  { id: "review", label: "En review" },
  { id: "completed", label: "Hecho" },
  { id: "canceled", label: "Cancelado" },
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
  0: "Sin prioridad",
  1: "Urgente",
  2: "Alta",
  3: "Media",
  4: "Baja",
};
