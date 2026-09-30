import { useEffect, useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import type { Priority, TaskStatus } from "../../domain/types";
import { Kbd } from "../../ui/Kbd";
import { PriorityBars, StatusRing } from "../tasks/bits";
import { BOARD_COLUMNS, HIDDEN_COLUMNS, PRIORITIES, PRIORITY_LABEL, STATUS_META } from "../tasks/status";
import { ACTION_LABEL, type CardAction, type CardModel } from "./TaskCard";

const ALL_STATUSES: TaskStatus[] = [HIDDEN_COLUMNS[0], ...BOARD_COLUMNS, HIDDEN_COLUMNS[1]];

interface Props {
  model: CardModel;
  /** Task key (`ND-12`) shown in the header and copied by "Copy ID". */
  taskId: string;
  at: { x: number; y: number };
  action: CardAction | null;
  /** An action is already in flight for this task: Run/Retry is disabled. */
  busy: boolean;
  onClose: () => void;
  onOpen: () => void;
  onAction: (a: CardAction) => void;
  onStatus: (s: TaskStatus) => void;
  onPriority: (p: Priority) => void;
  onCopyId: () => void;
  onDelete: () => void;
}

type Sub = "status" | "priority";

/** Right-click menu of a board card. Arrows move, → opens a submenu, ← / Esc go back. */
export function CardMenu({ model, taskId, at, action, busy, onClose, onOpen, onAction, onStatus, onPriority, onCopyId, onDelete }: Props) {
  const { task } = model;
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState(at);
  const [sub, setSub] = useState<Sub | null>(null);
  const [flip, setFlip] = useState(false);
  // Imported tasks don't accept a priority patch (the provider owns it).
  const canPriority = !task.source;

  // Keep it inside the window; submenus open to the left near the right edge.
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const r = el.getBoundingClientRect();
    const x = Math.max(8, Math.min(at.x, window.innerWidth - r.width - 8));
    const y = Math.max(8, Math.min(at.y, window.innerHeight - r.height - 8));
    setPos({ x, y });
    setFlip(x + r.width + 200 > window.innerWidth);
  }, [at]);

  // Same for an open submenu: shift it up if it would run past the bottom edge.
  useLayoutEffect(() => {
    const box = ref.current?.querySelector<HTMLElement>(".bd-ctx-sub");
    if (!box) return;
    box.style.top = "";
    const r = box.getBoundingClientRect();
    const over = r.bottom - (window.innerHeight - 8);
    if (over > 0) box.style.top = `${box.offsetTop - Math.min(over, r.top - 8)}px`;
  }, [sub, pos]);

  // Latest `onClose` without re-running the effect (it restores focus on cleanup).
  const closeRef = useRef(onClose);
  closeRef.current = onClose;

  useEffect(() => {
    const prev = document.activeElement as HTMLElement | null;
    ref.current?.querySelector<HTMLElement>(".menu-item")?.focus();
    const close = () => closeRef.current();
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) close();
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("resize", close);
    window.addEventListener("blur", close);
    document.addEventListener("scroll", close, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("resize", close);
      window.removeEventListener("blur", close);
      document.removeEventListener("scroll", close, true);
      if (prev?.isConnected) prev.focus();
    };
  }, []);

  const openSub = (s: Sub, focus: boolean) => {
    setSub(s);
    if (!focus) return;
    requestAnimationFrame(() => {
      const box = ref.current?.querySelector<HTMLElement>(".bd-ctx-sub");
      (box?.querySelector<HTMLElement>('[aria-checked="true"]') ?? box?.querySelector<HTMLElement>(".menu-item"))?.focus();
    });
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const inSub = !!(e.target as HTMLElement).closest(".bd-ctx-sub");
    const scope = inSub ? ref.current?.querySelector(".bd-ctx-sub") : ref.current;
    const items = [...(scope?.querySelectorAll<HTMLElement>(inSub ? ".menu-item" : ":scope > .bd-ctx-row > .menu-item:not(:disabled)") ?? [])];
    const i = items.indexOf(document.activeElement as HTMLElement);
    const back = () => {
      setSub(null);
      ref.current?.querySelector<HTMLElement>(`[data-sub="${sub}"]`)?.focus();
    };
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      if (inSub) back();
      else onClose();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      const n = e.key === "ArrowDown" ? (i + 1) % items.length : (i - 1 + items.length) % items.length;
      items[n]?.focus();
    } else if (e.key === "ArrowRight" && !inSub) {
      const s = (document.activeElement as HTMLElement | null)?.dataset.sub as Sub | undefined;
      if (s) {
        e.preventDefault();
        openSub(s, true);
      }
    } else if (e.key === "ArrowLeft" && inSub) {
      e.preventDefault();
      back();
    } else if (e.key === "Tab") {
      e.preventDefault();
      onClose();
    }
  };

  const run = (fn: () => void) => () => {
    onClose();
    fn();
  };

  const subMenu = (s: Sub) =>
    sub === s && (
      <div className={`menu bd-ctx-sub ${flip ? "flip" : ""}`} role="menu" aria-label={s === "status" ? "Status" : "Priority"}>
        {s === "status"
          ? ALL_STATUSES.map((st) => (
              <button
                key={st}
                type="button"
                role="menuitemradio"
                aria-checked={task.status === st}
                className="menu-item"
                onClick={run(() => onStatus(st))}
              >
                <span className="menu-mark" aria-hidden>{task.status === st ? "✓" : ""}</span>
                <StatusRing status={st} />
                {STATUS_META[st].label}
              </button>
            ))
          : PRIORITIES.map((p) => (
              <button
                key={p}
                type="button"
                role="menuitemradio"
                aria-checked={task.priority === p}
                className="menu-item"
                onClick={run(() => onPriority(p))}
              >
                <span className="menu-mark" aria-hidden>{task.priority === p ? "✓" : ""}</span>
                <PriorityBars priority={p} />
                {PRIORITY_LABEL[p]}
              </button>
            ))}
      </div>
    );

  const subItem = (s: Sub, label: string) => (
    <div className="bd-ctx-row" onMouseEnter={() => openSub(s, false)}>
      <button
        type="button"
        role="menuitem"
        className="menu-item"
        data-sub={s}
        aria-haspopup="menu"
        aria-expanded={sub === s}
        onClick={() => openSub(s, true)}
      >
        <span className="menu-mark" aria-hidden />
        <span className="bd-ctx-label">{label}</span>
        <span className="bd-ctx-caret" aria-hidden>▸</span>
      </button>
      {subMenu(s)}
    </div>
  );

  const item = (label: string, fn: () => void, opts: { kbd?: string; danger?: boolean; disabled?: boolean } = {}) => (
    <div className="bd-ctx-row" onMouseEnter={() => setSub(null)}>
      <button
        type="button"
        role="menuitem"
        className={`menu-item ${opts.danger ? "bd-ctx-danger" : ""}`}
        disabled={opts.disabled}
        onClick={run(fn)}
      >
        <span className="menu-mark" aria-hidden />
        <span className="bd-ctx-label">{label}</span>
        {opts.kbd && <Kbd>{opts.kbd}</Kbd>}
      </button>
    </div>
  );

  return (
    <div
      ref={ref}
      className="menu bd-ctx"
      role="menu"
      aria-label="Task actions"
      style={{ left: pos.x, top: pos.y }}
      onKeyDown={onKey}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
    >
      <div className="bd-ctx-head">
        <span className="mono">{taskId}</span>
        <span className="ellipsis">{task.title}</span>
      </div>
      {item("Open", onOpen, { kbd: "↵" })}
      {action && item(ACTION_LABEL[action], () => onAction(action), { disabled: busy })}
      <div role="separator" className="bd-ctx-sep" />
      {subItem("status", "Status")}
      {canPriority && subItem("priority", "Priority")}
      <div role="separator" className="bd-ctx-sep" />
      {item("Copy ID", onCopyId)}
      {item("Delete…", onDelete, { danger: true })}
    </div>
  );
}
