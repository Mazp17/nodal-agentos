import { useEffect, useRef, useState, type KeyboardEvent } from "react";

export interface FilterOption {
  value: string;
  label: string;
  /** Color del punto (variable CSS o color). */
  dot?: string;
}

interface Props {
  label: string;
  value: string | null;
  options: FilterOption[];
  onChange: (value: string | null) => void;
}

/** Chip de filtro con menú ("Repo", "Source", "Status", "Label"). */
export function FilterMenu({ label, value, options, onChange }: Props) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const current = options.find((o) => o.value === value);

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

  const all: (FilterOption | { value: null; label: string; dot?: undefined })[] = [{ value: null, label: "Any" }, ...options];

  return (
    <div className="bd-filter" ref={wrap}>
      <button
        ref={trigger}
        type="button"
        className={`bd-chip ${value ? "on" : ""}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {current ? `${label}: ${current.label}` : label}
      </button>
      {open && (
        <div className="menu bd-filter-menu" role="menu" aria-label={label} onKeyDown={onKey}>
          {all.map((o) => (
            <button
              key={o.value ?? "__any"}
              type="button"
              role="menuitemradio"
              aria-checked={o.value === value}
              className="menu-item"
              onClick={() => {
                onChange(o.value);
                close();
              }}
            >
              <span className="menu-mark" aria-hidden>{o.value === value ? "✓" : ""}</span>
              <span className="bd-filter-dot" style={{ background: o.dot ?? "transparent" }} aria-hidden />
              <span className="ellipsis">{o.label}</span>
            </button>
          ))}
          {options.length === 0 && <div className="menu-label">Nothing to filter by</div>}
        </div>
      )}
    </div>
  );
}
