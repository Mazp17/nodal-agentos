// STUB(F2-E): replaced by F (providers). Contrato mínimo que consume E: `SourceTab({ task })`.
import { createElement } from "react";
import type { Task } from "../../domain/types";

/** STUB(F2-E): replaced by F. Detalle de la issue vinculada de una tarea importada. */
export function SourceTab({ task }: { task: Task }) {
  const src = task.source;
  if (!src) return null;
  return createElement(
    "div",
    { className: "tp-section" },
    createElement("span", { className: "section-label" }, `${src.provider} · ${src.identifier}`),
    createElement("span", { className: "tp-muted" }, src.externalState?.name ?? "Unknown state"),
  );
}
