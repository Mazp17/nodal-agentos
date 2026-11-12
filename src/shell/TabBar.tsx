import { useEffect, useRef, type ReactNode } from "react";
import { ContextMenu } from "radix-ui";

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
  /** Actions after the tabs (a "+" right after them) and pinned to the right edge. */
  after?: ReactNode;
  end?: ReactNode;
}

/** Browser-like tabs over the main panel: chats in the Chat view, boards in Tasks. */
export function TabBar({ label, items, active, onPick, onClose, onCloseMany, after, end }: Props) {
  // Like an editor: the active tab (just opened or picked elsewhere) scrolls into view.
  const scroller = useRef<HTMLDivElement>(null);
  const count = items.length;
  useEffect(() => {
    scroller.current?.querySelector(".tab.on")?.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }, [active, count]);
  return (
    <div className="tabbar">
      <div ref={scroller} className="tabbar-scroll" role="tablist" aria-label={label}>
        {items.map((t, i) => {
          const on = t.id === active;
          const closable = items.filter((x) => x.closable);
          const right = items.slice(i + 1).filter((x) => x.closable);
          const others = closable.filter((x) => x.id !== t.id);
          const ids = (xs: TabItem[]) => xs.map((x) => x.id);
          return (
            <ContextMenu.Root key={t.id}>
            <ContextMenu.Trigger asChild>
            <div className={`tab ${on ? "on" : ""} ${t.dirty ? "dirty" : ""}`}>
              <button
                type="button"
                role="tab"
                aria-selected={on}
                className="tab-pick"
                title={t.label}
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
              </ContextMenu.Content>
            </ContextMenu.Portal>
            </ContextMenu.Root>
          );
        })}
      </div>
      {after}
      {end && <div className="tabbar-end">{end}</div>}
    </div>
  );
}
