import { useEffect, useId, useMemo, useRef, useState, type KeyboardEvent } from "react";
import type { ExecutorInfo } from "../../domain/api";
import { useExecutors } from "../../domain/hooks/tasks";
import type { Executor } from "../../domain/types";
import { ExecutorAvatar } from "./ExecutorAvatar";
import { CLAUDE, executorKey, executorLabel, sameExecutor } from "./executors";
import "./executors.css";

export interface ExecutorPickerProps {
  /** Suma los agentes y workflows del repo al catálogo global. */
  repoId: string | null;
  /** `null` = hereda (`inherited`). */
  value: Executor | null;
  onChange: (value: Executor | null) => void;
  /** Si viene, ofrece "Default · x" (valor `null`). */
  inherited?: Executor;
  /** Nombre accesible del botón. */
  label?: string;
  disabled?: boolean;
  /** Abre el menú hacia arriba (p. ej. al pie de un panel). */
  dropUp?: boolean;
}

interface Group {
  title: string;
  items: { executor: Executor | null; info: ExecutorInfo | null }[];
}

const SOURCE_LABEL: Record<string, string> = { user: "User", repo: "Repo", plugin: "Plugin" };

function matches(info: ExecutorInfo, q: string) {
  if (!q) return true;
  const hay = `${executorLabel(info.executor)} ${info.description ?? ""} ${info.source ?? ""}`.toLowerCase();
  return hay.includes(q);
}

/** Selector de ejecutor: agentes, workflows y Claude, cada uno con su fuente y descripción. */
export function ExecutorPicker({ repoId, value, onChange, inherited, label = "Executor", disabled, dropUp }: ExecutorPickerProps) {
  const { data: catalog, error } = useExecutors(repoId);
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const menuId = useId();

  const shown = value ?? inherited ?? CLAUDE;
  const known = !value || value.kind === "claude" || !catalog || catalog.some((i) => sameExecutor(i.executor, value));

  const groups = useMemo<Group[]>(() => {
    const ql = q.trim().toLowerCase();
    const list = (catalog ?? []).filter((i) => matches(i, ql));
    const agents = list.filter((i) => i.executor.kind === "agent");
    const workflows = list.filter((i) => i.executor.kind === "workflow");
    const claudeInfo = list.find((i) => i.executor.kind === "claude") ?? null;
    const out: Group[] = [];
    if (inherited && (!ql || `default ${executorLabel(inherited)}`.toLowerCase().includes(ql))) {
      out.push({ title: "Default", items: [{ executor: null, info: null }] });
    }
    if (agents.length) out.push({ title: "Agents", items: agents.map((info) => ({ executor: info.executor, info })) });
    if (workflows.length) out.push({ title: "Workflows", items: workflows.map((info) => ({ executor: info.executor, info })) });
    // Claude siempre está, aunque el catálogo no lo liste.
    if (claudeInfo || !ql || "claude".includes(ql)) {
      out.push({ title: "Claude", items: [{ executor: claudeInfo?.executor ?? CLAUDE, info: claudeInfo }] });
    }
    return out;
  }, [catalog, q, inherited]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    search.current?.focus();
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const close = () => {
    setOpen(false);
    setQ("");
    trigger.current?.focus();
  };
  const pick = (e: Executor | null) => {
    onChange(e);
    close();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(wrap.current?.querySelectorAll<HTMLElement>(".ex-item") ?? [])];
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      items[Math.min(items.length - 1, i + 1)]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      if (i <= 0) search.current?.focus();
      else items[i - 1]?.focus();
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  return (
    <div className="ex-picker" ref={wrap}>
      <button
        ref={trigger}
        type="button"
        className="ex-trigger"
        aria-label={`${label}: ${value ? executorLabel(shown) : `Default (${executorLabel(shown)})`}`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? menuId : undefined}
        disabled={disabled}
        onClick={(e) => {
          e.stopPropagation();
          setOpen(!open);
        }}
      >
        <ExecutorAvatar executor={shown} />
        <span className="ellipsis">{executorLabel(shown)}</span>
        {!value && inherited && <span className="ex-default">default</span>}
        {!known && <span className="ex-missing" title="Not found in this repo's catalog">not found</span>}
        <span className="ex-caret" aria-hidden>▼</span>
      </button>
      {open && (
        <div
          id={menuId}
          className={`menu ex-menu ${dropUp ? "ex-menu-up" : ""}`}
          role="menu"
          aria-label={label}
          onKeyDown={onKey}
          onClick={(e) => e.stopPropagation()}
        >
          <input
            ref={search}
            className="input ex-search"
            placeholder="Search agents and workflows"
            aria-label="Search executors"
            value={q}
            onChange={(e) => setQ(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                wrap.current?.querySelector<HTMLElement>(".ex-item")?.focus();
              }
            }}
          />
          {error && <div className="ex-note ex-error">{error}</div>}
          {!catalog && !error && <div className="ex-note">Loading executors…</div>}
          {groups.map((g) => (
            <div key={g.title} role="group" aria-label={g.title}>
              <div className="menu-label">{g.title}</div>
              {g.items.map(({ executor, info }) => {
                const ex = executor ?? inherited ?? CLAUDE;
                const checked = executor === null ? value === null : sameExecutor(value, executor);
                return (
                  <button
                    key={executor ? executorKey(executor) : "__default"}
                    type="button"
                    role="menuitemradio"
                    aria-checked={checked}
                    className="ex-item"
                    onClick={() => pick(executor)}
                  >
                    <span className="menu-mark" aria-hidden>{checked ? "✓" : ""}</span>
                    <ExecutorAvatar executor={ex} />
                    <span className="ex-item-body">
                      <span className="ex-item-head">
                        <span className="ex-item-name ellipsis">
                          {executor === null ? `Default · ${executorLabel(ex)}` : executorLabel(ex)}
                        </span>
                        {info?.source && <span className="ex-src">{SOURCE_LABEL[info.source] ?? info.source}</span>}
                        {info?.reviews && <span className="ex-src" title="Reviews its own work: skips the review gate">reviews</span>}
                      </span>
                      <span className="ex-item-desc">
                        {executor === null
                          ? "Inherited from the repo or project"
                          : ex.kind === "claude"
                            ? (info?.description ?? "A plain Claude session with the task as prompt")
                            : (info?.description ?? "No description")}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
          {catalog && groups.length === 0 && <div className="ex-note">No executors match.</div>}
        </div>
      )}
    </div>
  );
}
