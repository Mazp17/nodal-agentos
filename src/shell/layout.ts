// Editor groups of the chat view, laid out like an editor's split grid: a tree whose leaves are
// groups and whose inner nodes split their children in a row or a column.

export type SplitSide = "left" | "right" | "up" | "down";
export type Layout = { group: string } | { dir: "row" | "col"; children: Layout[] };

export const isLayout = (v: unknown): v is Layout => {
  if (!v || typeof v !== "object") return false;
  const n = v as Record<string, unknown>;
  if (typeof n.group === "string") return true;
  return (n.dir === "row" || n.dir === "col") && Array.isArray(n.children) && n.children.length > 0 && n.children.every(isLayout);
};

export type DropZone = SplitSide | "center";

/** Where a tab dropped at (x, y) on a group lands: an edge quarter splits, the middle moves in. */
export function dropZoneAt(r: { left: number; top: number; width: number; height: number }, x: number, y: number): DropZone {
  const fx = (x - r.left) / r.width;
  const fy = (y - r.top) / r.height;
  // The nearest edge wins, if within a quarter of the size.
  const edges: [DropZone, number][] = [["left", fx], ["right", 1 - fx], ["up", fy], ["down", 1 - fy]];
  const [zone, d] = edges.reduce((a, b) => (b[1] < a[1] ? b : a));
  return d < 0.25 ? zone : "center";
}

export const groupsOf = (l: Layout): string[] => ("group" in l ? [l.group] : l.children.flatMap(groupsOf));

/** Puts group `added` beside group `target`, on `side`. */
export function splitLayout(l: Layout, target: string, added: string, side: SplitSide): Layout {
  const dir = side === "left" || side === "right" ? "row" : "col";
  const before = side === "left" || side === "up";
  if ("group" in l) {
    if (l.group !== target) return l;
    const pair = before ? [{ group: added }, l] : [l, { group: added }];
    return { dir, children: pair };
  }
  const i = l.children.findIndex((c) => "group" in c && c.group === target);
  // Same direction as this split: a new sibling, not a nested split.
  if (i >= 0 && l.dir === dir) {
    const children = [...l.children];
    children.splice(before ? i : i + 1, 0, { group: added });
    return { dir, children };
  }
  return { dir: l.dir, children: l.children.map((c) => splitLayout(c, target, added, side)) };
}

/** Drops a group; a split left with one child becomes that child. `null` when nothing is left. */
export function removeFromLayout(l: Layout, id: string): Layout | null {
  if ("group" in l) return l.group === id ? null : l;
  const children = l.children.map((c) => removeFromLayout(c, id)).filter((c): c is Layout => c !== null);
  if (children.length === 0) return null;
  if (children.length === 1) return children[0]!;
  // A child split in the same direction folds into this one.
  return { dir: l.dir, children: children.flatMap((c) => ("dir" in c && c.dir === l.dir ? c.children : [c])) };
}
