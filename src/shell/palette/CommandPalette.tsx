import { useId, useState, type KeyboardEvent } from "react";
import { useFocusTrap } from "../../ui/useFocusTrap";
import "./palette.css";

export interface PaletteItem {
  id: string;
  kind: "Action" | "Run" | "Task";
  label: string;
  sub?: string;
  /** Texto extra para el filtro (p. ej. identifier + título). */
  keywords?: string;
  run: () => void;
}

interface Props {
  /** Acciones fijas; se filtran por texto. */
  actions: PaletteItem[];
  /** Resultados según la búsqueda (tareas y "Run X"). */
  search: (query: string) => PaletteItem[];
  onClose: () => void;
}

const MAX_ITEMS = 11;

export function CommandPalette({ actions, search, onClose }: Props) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const listId = useId();
  const q = query.trim().toLowerCase();

  const items = [
    ...actions.filter((a) => !q || `${a.label} ${a.keywords ?? ""}`.toLowerCase().includes(q)),
    ...search(q),
  ].slice(0, MAX_ITEMS);
  const active = Math.min(index, Math.max(0, items.length - 1));

  const run = (item: PaletteItem | undefined) => {
    if (!item) return;
    onClose();
    item.run();
  };

  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      setIndex(Math.min(items.length - 1, active + 1));
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      setIndex(Math.max(0, active - 1));
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(items[active]);
    }
  };

  return (
    <>
      <div className="palette-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="palette" role="dialog" aria-modal="true" aria-label="Command palette" tabIndex={-1}>
        <input
          data-autofocus
          className="palette-input"
          placeholder="Search tasks or type a command…"
          aria-label="Search tasks or type a command"
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setIndex(0);
          }}
          onKeyDown={onKey}
          role="combobox"
          aria-expanded="true"
          aria-controls={listId}
          aria-activedescendant={items[active] ? `${listId}-${active}` : undefined}
          aria-autocomplete="list"
          spellCheck={false}
        />
        <div className="palette-list" id={listId} role="listbox" aria-label="Results">
          {items.map((it, k) => (
            <div
              key={it.id}
              id={`${listId}-${k}`}
              role="option"
              aria-selected={k === active}
              className={`palette-item ${k === active ? "on" : ""}`}
              onMouseEnter={() => setIndex(k)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => run(it)}
            >
              <span className={`palette-kind palette-kind-${it.kind.toLowerCase()}`}>{it.kind}</span>
              <span className="palette-label ellipsis">{it.label}</span>
              {it.sub && <span className="palette-sub">{it.sub}</span>}
            </div>
          ))}
          {items.length === 0 && <div className="palette-empty">No matches.</div>}
        </div>
        <div className="palette-foot" aria-hidden>
          <span>↑↓ navigate</span>
          <span>↵ run</span>
          <span>esc close</span>
        </div>
      </div>
    </>
  );
}
