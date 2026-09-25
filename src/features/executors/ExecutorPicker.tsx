import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type CSSProperties, type KeyboardEvent } from "react";
import { createPortal } from "react-dom";
import type { ExecutorInfo } from "../../domain/api";
import { useExecutors } from "../../domain/hooks/store";
import type { Executor } from "../../domain/types";
import { FOCUSABLE } from "../../ui/useFocusTrap";
import { ExecutorAvatar } from "./ExecutorAvatar";
import { CLAUDE, executorKey, executorLabel, sameExecutor } from "./executors";
import "./executors.css";

export interface ExecutorPickerProps {
  /** Adds the repo's agents and workflows to the global catalog. */
  repoId: string | null;
  /** `null` = inherits (`inherited`). */
  value: Executor | null;
  onChange: (value: Executor | null) => void;
  /** When set, offers "Default · x" (value `null`). */
  inherited?: Executor;
  /** Accessible name of the button. */
  label?: string;
  disabled?: boolean;
  /** Prefers opening the menu upwards (e.g. at the foot of a panel). */
  dropUp?: boolean;
}

const MENU_W = 340;
const MENU_MAX_H = 360;
const GAP = 4;
const EDGE = 8;

/**
 * Fixed position of the menu next to the trigger. It lives in a portal on the body because
 * panels and dialogs clip it (overflow) and their `translate` breaks `position: fixed` inside.
 * Opens towards the side with more room when the preferred one doesn't fit, and stays in the window.
 */
function menuPosition(trigger: DOMRect, dropUp: boolean): CSSProperties {
  const below = window.innerHeight - trigger.bottom - GAP - EDGE;
  const above = trigger.top - GAP - EDGE;
  const up = dropUp ? above >= MENU_MAX_H || above >= below : below < MENU_MAX_H && above > below;
  const left = Math.max(EDGE, Math.min(trigger.left, window.innerWidth - MENU_W - EDGE));
  return up
    ? { left, bottom: window.innerHeight - trigger.top + GAP, maxHeight: Math.max(0, Math.min(MENU_MAX_H, above)) }
    : { left, top: trigger.bottom + GAP, maxHeight: Math.max(0, Math.min(MENU_MAX_H, below)) };
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

/** Executor picker: agents, workflows and Claude, each with its source and description. */
export function ExecutorPicker({ repoId, value, onChange, inherited, label = "Executor", disabled, dropUp }: ExecutorPickerProps) {
  const { data: catalog, error } = useExecutors(repoId);
  const [open, setOpen] = useState(false);
  const [q, setQ] = useState("");
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const menu = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState<CSSProperties | null>(null);
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
    // Claude is always there, even if the catalog doesn't list it.
    if (claudeInfo || !ql || "claude".includes(ql)) {
      out.push({ title: "Claude", items: [{ executor: claudeInfo?.executor ?? CLAUDE, info: claudeInfo }] });
    }
    return out;
  }, [catalog, q, inherited]);

  useLayoutEffect(() => {
    if (!open) {
      setPos(null);
      return;
    }
    const place = () => {
      const r = trigger.current?.getBoundingClientRect();
      if (!r) return;
      // If the trigger scrolls out of view, the menu has nothing to anchor to.
      if (r.bottom < 0 || r.top > window.innerHeight) setOpen(false);
      else setPos(menuPosition(r, !!dropUp));
    };
    const onScroll = (e: Event) => {
      if (!menu.current?.contains(e.target as Node)) place();
    };
    place();
    // Capture: follows the trigger when its containing panel scrolls.
    window.addEventListener("resize", place);
    window.addEventListener("scroll", onScroll, true);
    return () => {
      window.removeEventListener("resize", place);
      window.removeEventListener("scroll", onScroll, true);
    };
  }, [open, dropUp]);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node;
      if (!wrap.current?.contains(t) && !menu.current?.contains(t)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const placed = pos !== null;
  useEffect(() => {
    if (placed) search.current?.focus();
  }, [placed]);

  const close = () => {
    setOpen(false);
    setQ("");
    trigger.current?.focus();
  };
  /**
   * Tab from the menu follows the trigger's order: the menu lives in a portal, so native
   * Tab would go to the end of the body. Wraps within the enclosing modal dialog.
   */
  const tabFrom = (back: boolean) => {
    const t = trigger.current;
    setOpen(false);
    setQ("");
    if (!t) return;
    const scope = t.closest<HTMLElement>('[aria-modal="true"]') ?? document.body;
    const list = [...scope.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
      (el) => el.getClientRects().length > 0 && !menu.current?.contains(el),
    );
    const i = list.indexOf(t);
    if (i < 0) return t.focus();
    const next = back ? list[i - 1] : list[i + 1];
    (next ?? (scope === document.body ? t : list[back ? list.length - 1 : 0]) ?? t).focus();
  };

  const pick = (e: Executor | null) => {
    onChange(e);
    close();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(menu.current?.querySelectorAll<HTMLElement>(".ex-item") ?? [])];
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
      // Don't propagate: the dialog's focus trap would move focus again.
      e.preventDefault();
      e.stopPropagation();
      tabFrom(e.shiftKey);
    }
  };

  return (
    <div className="ex-picker" ref={wrap}>
      <button
        ref={trigger}
        type="button"
        className="ex-trigger"
        aria-label={`${label}: ${value ? executorLabel(shown) : `Default (${executorLabel(shown)})`}`}
        aria-haspopup="dialog"
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
      {open && pos && createPortal(
        <div
          ref={menu}
          id={menuId}
          className="menu ex-menu"
          style={pos}
          role="dialog"
          // Outside the parent dialog (aria-modal): without this VoiceOver hides it.
          aria-modal="true"
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
                menu.current?.querySelector<HTMLElement>(".ex-item")?.focus();
              }
            }}
          />
          <div role="menu" aria-label={label}>
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
        </div>,
        document.body,
      )}
    </div>
  );
}
