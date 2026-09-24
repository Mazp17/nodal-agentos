// Selector compacto de ejecutor por defecto (proyecto y repo). El selector rico con
// descripciones es de Tasks (E); acá alcanza con un <select> agrupado.

import { useEffect, useState } from "react";
import { listExecutors, type ExecutorInfo } from "../../domain/api";
import type { Executor } from "../../domain/types";

export const executorKey = (e: Executor): string =>
  e.kind === "agent" ? `agent:${e.source}:${e.name}` : e.kind === "workflow" ? `workflow:${e.name}` : "claude";

export const executorLabel = (e: Executor): string =>
  e.kind === "agent" ? e.name : e.kind === "workflow" ? `/${e.name}` : "Claude";

export function useExecutors(repoId: string | null) {
  const [list, setList] = useState<ExecutorInfo[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let alive = true;
    listExecutors(repoId)
      .then((l) => alive && setList(l))
      .catch((e) => alive && setError(String(e)));
    return () => {
      alive = false;
    };
  }, [repoId]);
  return { list, error };
}

/**
 * `value: null` = hereda (`inheritLabel`). Si el valor guardado ya no está en el catálogo
 * (agente borrado), igual se muestra para no perderlo en silencio.
 */
export function ExecutorSelect({
  id,
  value,
  onChange,
  repoId,
  inheritLabel,
}: {
  id?: string;
  value: Executor | null;
  onChange: (e: Executor | null) => void;
  repoId: string | null;
  inheritLabel: string;
}) {
  const { list, error } = useExecutors(repoId);
  const items = list ?? [];
  const byKey = new Map(items.map((i) => [executorKey(i.executor), i.executor]));
  const current = value ? executorKey(value) : "";
  const missing = value && list && !byKey.has(current);
  const agents = items.filter((i) => i.executor.kind === "agent");
  const workflows = items.filter((i) => i.executor.kind === "workflow");

  return (
    <>
    <select
      id={id}
      className="select executor-select"
      value={current}
      disabled={!list && !error}
      title={error ?? undefined}
      onChange={(e) => {
        const k = e.target.value;
        if (k === "") onChange(null);
        else if (k === "claude") onChange({ kind: "claude" });
        else onChange(byKey.get(k) ?? value);
      }}
    >
      <option value="">{inheritLabel}</option>
      <option value="claude">Claude</option>
      {missing && value && <option value={current}>{executorLabel(value)} (not found)</option>}
      {agents.length > 0 && (
        <optgroup label="Agents">
          {agents.map((i) => (
            <option key={executorKey(i.executor)} value={executorKey(i.executor)}>
              {executorLabel(i.executor)}
              {i.source && i.source !== "user" ? ` · ${i.source}` : ""}
            </option>
          ))}
        </optgroup>
      )}
      {workflows.length > 0 && (
        <optgroup label="Workflows">
          {workflows.map((i) => (
            <option key={executorKey(i.executor)} value={executorKey(i.executor)}>
              {executorLabel(i.executor)}
            </option>
          ))}
        </optgroup>
      )}
    </select>
    {error && (
      <span className="field-error" role="alert">
        Couldn't list agents and workflows: {error}
      </span>
    )}
    </>
  );
}
