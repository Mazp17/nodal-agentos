import type { Executor } from "../../domain/types";
import { Avatar } from "../../ui/Avatar";
import { executorKindLabel, executorLabel } from "./executors";
import "./executors.css";

/** Ícono del ejecutor, como el assignee de Linear: iniciales (agente) o glifo (workflow, Claude). */
export function ExecutorAvatar({ executor, size = "sm" }: { executor: Executor; size?: "sm" | "md" }) {
  if (executor.kind === "agent") return <Avatar name={executor.name} size={size} />;
  const title = `${executorKindLabel(executor)} · ${executorLabel(executor)}`;
  return (
    <span className={`ex-icon ex-icon-${size} ex-icon-${executor.kind}`} title={title} aria-hidden>
      {executor.kind === "workflow" ? (
        <svg viewBox="0 0 12 12" width="100%" height="100%">
          <circle cx="3" cy="3" r="1.6" fill="currentColor" />
          <circle cx="9" cy="6" r="1.6" fill="currentColor" />
          <circle cx="3" cy="9" r="1.6" fill="currentColor" />
          <path d="M4.3 3.6 7.7 5.4M4.3 8.4 7.7 6.6" stroke="currentColor" strokeWidth="1" fill="none" />
        </svg>
      ) : (
        <svg viewBox="0 0 12 12" width="100%" height="100%">
          <path
            d="M6 1.2v9.6M1.2 6h9.6M2.6 2.6l6.8 6.8M9.4 2.6 2.6 9.4"
            stroke="currentColor"
            strokeWidth="1.4"
            strokeLinecap="round"
          />
        </svg>
      )}
    </span>
  );
}

/** Avatar + nombre, para listas y cadenas de pasos. */
export function ExecutorName({ executor, size = "sm" }: { executor: Executor; size?: "sm" | "md" }) {
  return (
    <span className="ex-name">
      <ExecutorAvatar executor={executor} size={size} />
      <span className="ellipsis">{executorLabel(executor)}</span>
    </span>
  );
}
