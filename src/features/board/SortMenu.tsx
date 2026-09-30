import { useEffect, useRef, useState, type KeyboardEvent } from "react";

export type BoardSort = "last-run" | "manual";

const OPTIONS: { value: BoardSort; label: string; hint: string }[] = [
  { value: "last-run", label: "Last run", hint: "Most recently run first; the rest keep their manual order" },
  { value: "manual", label: "Manual", hint: "The order you set by dragging cards" },
];

/** "Order ▾" picker in the board toolbar. */
export function SortMenu({ value, onChange }: { value: BoardSort; onChange: (v: BoardSort) => void }) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const current = OPTIONS.find((o) => o.value === value) ?? OPTIONS[0]!;

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    wrap.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(wrap.current?.querySelectorAll<HTMLElement>(".menu-item") ?? [])];
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
      items[Math.max(0, i - 1)]?.focus();
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  return (
    <div className="bd-filter" ref={wrap}>
      <button
        ref={trigger}
        type="button"
        className="btn btn-sm btn-ghost bd-sort"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className="bd-sort-label">Order</span>
        <span>{current.label}</span>
        <span className="bd-caret" aria-hidden>▾</span>
      </button>
      {open && (
        <div className="menu bd-filter-menu bd-sort-menu" role="menu" aria-label="Order" onKeyDown={onKey}>
          {OPTIONS.map((o) => (
            <button
              key={o.value}
              type="button"
              role="menuitemradio"
              aria-checked={o.value === value}
              className="menu-item bd-sort-item"
              onClick={() => {
                onChange(o.value);
                close();
              }}
            >
              <span className="menu-mark" aria-hidden>{o.value === value ? "✓" : ""}</span>
              <span className="bd-sort-text">
                <span>{o.label}</span>
                <span className="bd-sort-hint">{o.hint}</span>
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
