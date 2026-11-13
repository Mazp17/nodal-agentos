// Drag handle on a side panel's inner edge: resizes it between `min` and `max`, and dragging it
// past half of `min` toward the window edge collapses the panel (dragging back reopens it).

export interface ResizerProps {
  /** Edge of the panel the handle sits on: "right" for a left sidebar, "left" for a right panel. */
  edge: "left" | "right";
  label: string;
  width: number;
  min: number;
  max: number;
  /** Width a double-click goes back to. */
  initial: number;
  onWidth: (w: number) => void;
  onCollapse: (collapsed: boolean) => void;
}

export const clampWidth = (w: number, min: number, max: number) => Math.max(min, Math.min(max, Math.round(w)));

/** Width for a pointer at `x`, or `null` when the panel should collapse. */
export function dragWidth(x: number, anchor: number, edge: "left" | "right", min: number, max: number): number | null {
  const raw = edge === "right" ? x - anchor : anchor - x;
  return raw < min / 2 ? null : clampWidth(raw, min, max);
}

export function Resizer({ edge, label, width, min, max, initial, onWidth, onCollapse }: ResizerProps) {
  const start = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0) return;
    e.preventDefault();
    const rect = e.currentTarget.parentElement!.getBoundingClientRect();
    // The panel's fixed side: it stays put while the panel grows, shrinks or collapses.
    const anchor = edge === "right" ? rect.left : rect.right;
    // Window listeners: the handle unmounts with the panel when a drag collapses it.
    const move = (ev: PointerEvent) => {
      const w = dragWidth(ev.clientX, anchor, edge, min, max);
      if (w === null) onCollapse(true);
      else {
        onCollapse(false);
        onWidth(w);
      }
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      window.removeEventListener("pointercancel", up);
      document.body.classList.remove("resizing-col");
    };
    document.body.classList.add("resizing-col");
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
    window.addEventListener("pointercancel", up);
  };
  return (
    <div
      role="separator"
      className={`side-resizer at-${edge}`}
      aria-orientation="vertical"
      aria-label={label}
      aria-valuemin={min}
      aria-valuemax={max}
      aria-valuenow={width}
      tabIndex={0}
      title="Drag to resize, to the edge to close; double-click to reset"
      onPointerDown={start}
      onDoubleClick={() => onWidth(initial)}
      onKeyDown={(e) => {
        const step = (e.shiftKey ? 32 : 8) * (edge === "right" ? 1 : -1);
        if (e.key === "ArrowLeft") onWidth(clampWidth(width - step, min, max));
        else if (e.key === "ArrowRight") onWidth(clampWidth(width + step, min, max));
        else return;
        e.preventDefault();
      }}
    />
  );
}
