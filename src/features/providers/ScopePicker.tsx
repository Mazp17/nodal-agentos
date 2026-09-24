import type { ScopeRef } from "../../domain/types";

const KIND_LABEL: Record<string, string> = { team: "Teams", project: "Projects" };

/** Selector de team/proyecto del proveedor, agrupado por tipo. */
export function ScopePicker({
  scopes,
  loading,
  error,
  value,
  onChange,
  allowNone,
  id,
}: {
  scopes: ScopeRef[] | null;
  loading: boolean;
  error: string | null;
  value: string | null;
  onChange: (id: string | null) => void;
  /** Texto de la opción vacía; sin él, la vacía es "Pick a team or project". */
  allowNone?: string;
  id?: string;
}) {
  if (error) return <span className="pv-hint pv-hint-error">Could not load teams and projects. {error}</span>;
  if (!scopes) return <span className="pv-hint">{loading ? "Loading teams and projects…" : "No teams or projects."}</span>;
  const kinds = [...new Set(scopes.map((s) => s.kind))];
  return (
    <select
      id={id}
      className="input pv-select"
      value={value ?? ""}
      onChange={(e) => onChange(e.target.value || null)}
      aria-label={id ? undefined : "Team or project"}
    >
      <option value="">{allowNone ?? "Pick a team or project"}</option>
      {kinds.map((k) => (
        <optgroup key={k} label={KIND_LABEL[k] ?? k}>
          {scopes
            .filter((s) => s.kind === k)
            .map((s) => (
              <option key={s.id} value={s.id}>
                {s.name}
              </option>
            ))}
        </optgroup>
      ))}
    </select>
  );
}
