import { Fragment, useEffect, useRef, useState, type ReactNode } from "react";
import { ContextMenu } from "radix-ui";
import type { SplitSide } from "./layout";

const SPLITS: { side: SplitSide; label: string }[] = [
  { side: "right", label: "Split right" },
  { side: "left", label: "Split left" },
  { side: "down", label: "Split down" },
  { side: "up", label: "Split up" },
];

export interface TabItem {
  id: string;
  label: string;
  /** Project color dot; none for "All projects". */
  color?: string;
  closable: boolean;
  /** A file tab: a file icon instead of the project dot. */
  file?: boolean;
  /** Unsaved edits: a dot where the close button sits (it shows on hover). */
  dirty?: boolean;
  /** Shown as a pulsing dot (a chat answering). */
  live?: boolean;
  unread?: boolean;
}

interface Props {
  label: string;
  items: TabItem[];
  active: string | null;
  onPick: (id: string) => void;
  onClose: (id: string) => void;
  /** Closes several tabs at once (the right-click menu). */
  onCloseMany: (ids: string[]) => void;
  /** Moves a tab into a new editor group on that side (the right-click menu). */
  onSplit?: (id: string, side: SplitSide) => void;
  /** Makes the tabs draggable: a tab (from any bar) dropped here at `index`, counted without it. */
  onDropTab?: (tab: string, index: number) => void;
  /** Actions after the tabs (a "+" right after them) and pinned to the right edge. */
  after?: ReactNode;
  end?: ReactNode;
}

/** Drag data type of a tab being dragged (its id). */
export const TAB_DRAG = "application/x-nodal-tab";
/** The tab being dragged, shared by every bar so the one it hovers can draw its ghost. */
let dragInfo: { id: string; label: string; width: number } | null = null;
export const isTabDrag =(e: React.DragEvent) => e.dataTransfer.types.includes(TAB_DRAG);

/** Browser-like tabs over the main panel: chats in the Chat view, boards in Tasks. */
export function TabBar({ label, items, active, onPick, onClose, onCloseMany, onSplit, onDropTab, after, end }: Props) {
  // Like an editor: the active tab (just opened or picked elsewhere) scrolls into view.
  const scroller = useRef<HTMLDivElement>(null);
  const count = items.length;
  useEffect(() => {
    scroller.current?.querySelector(".tab.on")?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }, [active, count]);
  // Where a dragged tab would land: before the tab at this position (`count`: at the end).
  const [over, setOver] = useState<number | null>(null);
  const [dragged, setDragged] = useState<string | null>(null);
  const indexAt = (x: number) => {
    const tabs = [...(scroller.current?.querySelectorAll<HTMLElement>(".tab:not(.tab-ghost)") ?? [])];
    const i = tabs.findIndex((el) => {
      const r = el.getBoundingClientRect();
      return x < r.left + r.width / 2;
    });
    return i < 0 ? tabs.length : i;
  };
  const drop = onDropTab && {
    onDragOver: (e: React.DragEvent) => {
      if (!isTabDrag(e)) return;
      e.preventDefault();
      e.dataTransfer.dropEffect = "move";
      setOver(indexAt(e.clientX));
    },
    onDragLeave: (e: React.DragEvent) => {
      if (!e.currentTarget.contains(e.relatedTarget as Node | null)) setOver(null);
    },
    onDrop: (e: React.DragEvent) => {
      const tab = e.dataTransfer.getData(TAB_DRAG);
      setOver(null);
      if (!tab) return;
      e.preventDefault();
      const at = indexAt(e.clientX);
      // The index skips the dragged tab itself when it sits left of the drop point.
      const from = items.findIndex((t) => t.id === tab);
      onDropTab(tab, from >= 0 && from < at ? at - 1 : at);
    },
  };
  // A faded copy of the dragged tab where it would land; none where it would stay put.
  const from = dragInfo ? items.findIndex((t) => t.id === dragInfo!.id) : -1;
  const ghost = dragInfo && !(from >= 0 && (over === from || over === from + 1)) && (
    <div className="tab tab-ghost" style={{ width: dragInfo.width }} aria-hidden>
      <span className="ellipsis">{dragInfo.label}</span>
    </div>
  );
  return (
    <div className="tabbar" {...drop}>
      <div ref={scroller} className="tabbar-scroll" role="tablist" aria-label={label}>
        {items.map((t, i) => {
          const on = t.id === active;
          const closable = items.filter((x) => x.closable);
          const right = items.slice(i + 1).filter((x) => x.closable);
          const others = closable.filter((x) => x.id !== t.id);
          const ids = (xs: TabItem[]) => xs.map((x) => x.id);
          return (
            <Fragment key={t.id}>
            {over === i && ghost}
            <ContextMenu.Root>
            <ContextMenu.Trigger asChild>
            <div className={`tab ${on ? "on" : ""} ${t.dirty ? "dirty" : ""} ${dragged === t.id ? "dragging" : ""}`}>
              <button
                type="button"
                role="tab"
                aria-selected={on}
                className="tab-pick"
                title={t.label}
                // On the button itself: WebKit never starts a drag from a <button> otherwise.
                draggable={!!onDropTab}
                onDragStart={(e) => {
                  e.dataTransfer.setData(TAB_DRAG, t.id);
                  e.dataTransfer.effectAllowed = "move";
                  const tabEl = e.currentTarget.parentElement!;
                  const r = tabEl.getBoundingClientRect();
                  e.dataTransfer.setDragImage(tabEl, e.clientX - r.left, e.clientY - r.top);
                  dragInfo = { id: t.id, label: t.label, width: r.width };
                  setDragged(t.id);
                }}
                onDragEnd={() => {
                  dragInfo = null;
                  setDragged(null);
                  setOver(null);
                }}
                onClick={() => onPick(t.id)}
                onAuxClick={(e) => {
                  if (e.button === 1 && t.closable) onClose(t.id);
                }}
              >
                {t.file ? (
                  <svg className="tab-file-icon" width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" aria-hidden focusable="false">
                    <path d="M3.5 1.5h4.5l3 3v8h-7.5zM8 1.5v3h3" />
                  </svg>
                ) : t.color ? (
                  <span className="project-dot" style={{ ["--project-color" as string]: t.color }} aria-hidden />
                ) : (
                  <span className="tab-dot-all" aria-hidden />
                )}
                <span className={`ellipsis ${t.unread ? "tab-unread" : ""}`}>{t.label}</span>
                {t.dirty && <span className="sr-only">, unsaved</span>}
                {t.live && <span className="dot dot-sm pulse tone-accent" role="img" aria-label="Answering" />}
              </button>
              {t.closable && (
                <button type="button" className="icon-btn tab-close" aria-label={`Close ${t.label}`} title="Close" onClick={() => onClose(t.id)}>
                  <svg width="9" height="9" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" aria-hidden focusable="false">
                    <path d="M2 2l6 6M8 2l-6 6" />
                  </svg>
                </button>
              )}
            </div>
            </ContextMenu.Trigger>
            <ContextMenu.Portal>
              <ContextMenu.Content className="menu tab-menu" collisionPadding={8}>
                <ContextMenu.Item className="menu-item" disabled={!t.closable} onSelect={() => onClose(t.id)}>
                  Close
                </ContextMenu.Item>
                <ContextMenu.Item className="menu-item" disabled={others.length === 0} onSelect={() => onCloseMany(ids(others))}>
                  Close others
                </ContextMenu.Item>
                <ContextMenu.Item className="menu-item" disabled={right.length === 0} onSelect={() => onCloseMany(ids(right))}>
                  Close to the right
                </ContextMenu.Item>
                <ContextMenu.Separator className="menu-sep" />
                <ContextMenu.Item className="menu-item" disabled={closable.length === 0} onSelect={() => onCloseMany(ids(closable))}>
                  Close all
                </ContextMenu.Item>
                {onSplit && (
                  <>
                    <ContextMenu.Separator className="menu-sep" />
                    {SPLITS.map((s) => (
                      <ContextMenu.Item key={s.side} className="menu-item" onSelect={() => onSplit(t.id, s.side)}>
                        {s.label}
                        {s.side === "right" && <span className="menu-kbd">⌘\</span>}
                      </ContextMenu.Item>
                    ))}
                  </>
                )}
              </ContextMenu.Content>
            </ContextMenu.Portal>
            </ContextMenu.Root>
            </Fragment>
          );
        })}
        {over === count && ghost}
      </div>
      {after}
      {end && <div className="tabbar-end">{end}</div>}
    </div>
  );
}
