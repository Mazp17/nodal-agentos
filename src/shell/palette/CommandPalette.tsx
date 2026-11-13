import { useEffect, useId, useState, type KeyboardEvent } from "react";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { Kbd, KbdGroup } from "../../ui/Kbd";
import "./palette.css";

export interface PaletteItem {
  id: string;
  kind: "Action" | "Task" | "Chat" | "File";
  label: string;
  sub?: string;
  /** Keyboard shortcut shown on the right ("⌘,"). */
  shortcut?: string;
  /** Extra text for the filter (e.g. identifier + title). */
  keywords?: string;
  run: () => void;
}

interface Props {
  /** Fixed actions; filtered by text. */
  actions: PaletteItem[];
  /** Results for the query (tasks and chats). */
  search: (query: string) => PaletteItem[];
  onClose: () => void;
}

const MAX_ITEMS = 18;
/** Result groups, in the order they're listed. */
const GROUPS: { kind: PaletteItem["kind"]; title: string }[] = [
  { kind: "Chat", title: "Chats" },
  { kind: "File", title: "Files" },
  { kind: "Task", title: "Tasks" },
  { kind: "Action", title: "Actions" },
];

export function CommandPalette({ actions, search, onClose }: Props) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const listId = useId();
  const q = query.trim().toLowerCase();

  const found = [...search(q), ...actions.filter((a) => !q || `${a.label} ${a.keywords ?? ""}`.toLowerCase().includes(q))];
  // Grouped by kind; arrow keys walk the flat list in the same order.
  const items = GROUPS.flatMap((g) => found.filter((it) => it.kind === g.kind)).slice(0, MAX_ITEMS);
  const active = Math.min(index, Math.max(0, items.length - 1));

  // With the arrow keys, the active option stays in view.
  useEffect(() => {
    document.getElementById(`${listId}-${active}`)?.scrollIntoView({ block: "nearest" });
  }, [active, listId]);

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
          placeholder="Search chats, files, tasks or actions…"
          aria-label="Search chats, files, tasks or actions"
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
            <div key={it.id} role="presentation">
            {(k === 0 || items[k - 1]!.kind !== it.kind) && (
              <div className="palette-group" role="presentation">
                {GROUPS.find((g) => g.kind === it.kind)!.title}
              </div>
            )}
            <div
              id={`${listId}-${k}`}
              role="option"
              aria-selected={k === active}
              className={`palette-item ${k === active ? "on" : ""}`}
              onMouseEnter={() => setIndex(k)}
              onMouseDown={(e) => e.preventDefault()}
              onClick={() => run(it)}
            >
              <span className={`palette-icon palette-icon-${it.kind.toLowerCase()}`} aria-hidden>
                <KindIcon kind={it.kind} />
              </span>
              <span className="palette-label ellipsis">{it.label}</span>
              {it.sub && <span className="palette-sub ellipsis">{it.sub}</span>}
              {it.shortcut && <Kbd aria-hidden>{it.shortcut}</Kbd>}
            </div>
            </div>
          ))}
          {items.length === 0 && <div className="palette-empty">No matches.</div>}
        </div>
        <div className="palette-foot" aria-hidden>
          <span>
            <KbdGroup>
              <Kbd>↑</Kbd>
              <Kbd>↓</Kbd>
            </KbdGroup>{" "}
            navigate
          </span>
          <span>
            <Kbd>↵</Kbd> open
          </span>
          <span>
            <Kbd>esc</Kbd> close
          </span>
        </div>
      </div>
    </>
  );
}

function KindIcon({ kind }: { kind: PaletteItem["kind"] }) {
  return (
    <svg width="15" height="15" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" focusable="false">
      {kind === "Chat" && <path d="M3.5 2h7A1.5 1.5 0 0 1 12 3.5v5A1.5 1.5 0 0 1 10.5 10H6.5L4 12v-2h-.5A1.5 1.5 0 0 1 2 8.5v-5A1.5 1.5 0 0 1 3.5 2z" />}
      {kind === "File" && <path d="M3.5 1.5h4.5l3 3v8h-7.5z M8 1.5v3h3" />}
      {kind === "Task" && (
        <>
          <circle cx="7" cy="7" r="5" />
          <path d="M4.8 7l1.5 1.5L9.3 5.5" />
        </>
      )}
      {kind === "Action" && <path d="M7.5 1.5L3 8h4l-.5 4.5L11 6H7z" />}
    </svg>
  );
}
