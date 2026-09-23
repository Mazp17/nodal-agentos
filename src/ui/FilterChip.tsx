import { useEffect, useRef, useState, type KeyboardEvent } from "react";

export interface ChipOption {
  value: string;
  label: string;
}

interface Props {
  label: string;
  value: string | null;
  options: ChipOption[];
  onChange: (value: string | null) => void;
  /** Texto de la opción "sin filtro"; `null` la oculta (hay que elegir una). */
  anyLabel?: string | null;
}

/** Chip de filtro con menú desplegable (Team / Project / Assignee). */
export function FilterChip({ label, value, options, onChange, anyLabel = "Any" }: Props) {
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
    // Enfoca la opción elegida (o "Any") al abrir.
    wrap.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };
  const pick = (v: string | null) => {
    onChange(v);
    close();
  };

  const onMenuKey = (e: KeyboardEvent<HTMLDivElement>) => {
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
    <div className="chip-wrap" ref={wrap}>
      <button
        ref={trigger}
        type="button"
        className={`chip ${value ? "chip-on" : ""}`}
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        {current ? `${label}: ${current.label}` : label}
        <span className="chip-caret" aria-hidden>▼</span>
      </button>
      {open && (
        <div className="menu chip-menu" role="menu" aria-label={label} onKeyDown={onMenuKey}>
          {[...(anyLabel === null ? [] : [{ value: null, label: anyLabel }]), ...options].map((o: { value: string | null; label: string }) => (
            <button
              key={o.value ?? "__any"}
              type="button"
              role="menuitemradio"
              aria-checked={o.value === value}
              className="menu-item"
              onClick={() => pick(o.value)}
            >
              <span className="menu-mark" aria-hidden>{o.value === value ? "✓" : ""}</span>
              <span className="ellipsis">{o.label}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
