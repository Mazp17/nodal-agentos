// Open tabs: chats and files (across projects, in editor groups) and boards. Persisted in
// `nodal.tabs` so the workspace comes back as it was.

import { useCallback, useEffect, useMemo, useState } from "react";
import { readJsonPref, writePref } from "./storage";
import { groupsOf, isLayout, removeFromLayout, splitLayout, type Layout, type SplitSide } from "./layout";

/** A chat tab is a chat id, or `new:<projectId>` for a chat not sent yet (one per project). */
export const draftTab = (projectId: string) => `new:${projectId}`;
export const draftProject = (tab: string): string | null => (tab.startsWith("new:") ? tab.slice(4) : null);

/** Where a file tab's file lives: the repo's checkout (`worktree: null`) or one of its worktrees. */
export interface FileRef {
  repoId: string;
  worktree: string | null;
  path: string;
  diff: boolean;
  /** A `commit:` tab: `path` is the commit's sha. */
  commit: boolean;
}

/** A file tab in the chat tab bar: `file:<repoId>:<worktree, URI-encoded>:<path>`, or `diff:…`
 * for the file's uncommitted changes. */
export const fileTab = (repoId: string, path: string, diff = false, worktree: string | null = null) =>
  `${diff ? "diff" : "file"}:${repoId}:${encodeURIComponent(worktree ?? "")}:${path}`;
/** A commit tab: one commit's changes, `commit:<repoId>:<worktree, URI-encoded>:<sha>`. */
export const commitTab = (repoId: string, sha: string, worktree: string | null = null) =>
  `commit:${repoId}:${encodeURIComponent(worktree ?? "")}:${sha}`;
export const parseFileTab = (tab: string): FileRef | null => {
  const kind = tab.slice(0, tab.indexOf(":"));
  if (kind !== "file" && kind !== "diff" && kind !== "commit") return null;
  const start = kind.length + 1;
  const i = tab.indexOf(":", start);
  const j = i < 0 ? -1 : tab.indexOf(":", i + 1);
  if (j < 0) return null;
  const wt = decodeURIComponent(tab.slice(i + 1, j));
  return { repoId: tab.slice(start, i), worktree: wt || null, path: tab.slice(j + 1), diff: kind === "diff", commit: kind === "commit" };
};

/** One editor group: its own row of chat/file tabs and the one showing. */
export interface Group {
  id: string;
  chats: string[];
  chat: string | null;
}

export interface State {
  groups: Group[];
  layout: Layout;
  /** The group last used: new tabs open there, and its active tab is "the" active tab. */
  focus: string;
  /** Project boards open next to the pinned "All projects". */
  boards: string[];
}

const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");
const isGroup = (v: unknown): v is Group => {
  if (!v || typeof v !== "object") return false;
  const g = v as Record<string, unknown>;
  return typeof g.id === "string" && isStrings(g.chats) && (g.chat === null || typeof g.chat === "string");
};
const single = (chats: string[], chat: string | null, boards: string[]): State => ({
  groups: [{ id: "g1", chats, chat }],
  layout: { group: "g1" },
  focus: "g1",
  boards,
});

/** Saved tabs, including the pre-groups shape (`{ chats, chat, boards }`), else none. */
export function loadState(v: unknown): State {
  const s = (v && typeof v === "object" ? v : {}) as Record<string, unknown>;
  const boards = isStrings(s.boards) ? s.boards : [];
  if (Array.isArray(s.groups) && s.groups.length > 0 && s.groups.every(isGroup) && isLayout(s.layout)) {
    const ids = s.groups.map((g) => g.id);
    const inLayout = groupsOf(s.layout);
    if (inLayout.length === ids.length && inLayout.every((id) => ids.includes(id))) {
      return { groups: s.groups, layout: s.layout, focus: typeof s.focus === "string" && ids.includes(s.focus) ? s.focus : ids[0]!, boards };
    }
  }
  if (isStrings(s.chats)) return single(s.chats, typeof s.chat === "string" ? s.chat : null, boards);
  return single([], null, boards);
}

const focused = (s: State): Group => s.groups.find((g) => g.id === s.focus) ?? s.groups[0]!;

/** Drops tabs from every group. Emptied groups close (the last one stays); a group's active tab
 * that goes becomes its nearest tab to the left, else its first. */
export function dropTabs(s: State, drop: (tab: string) => boolean, skip?: string): State {
  let groups = s.groups.map((g) => {
    if (g.id === skip) return g;
    const chats = g.chats.filter((t) => !drop(t));
    if (chats.length === g.chats.length) return g;
    let chat = g.chat;
    if (chat && drop(chat)) {
      const i = g.chats.indexOf(chat);
      chat = [...g.chats.slice(0, i)].reverse().find((t) => !drop(t)) ?? chats[0] ?? null;
    }
    return { ...g, chats, chat };
  });
  let layout = s.layout;
  let focus = s.focus;
  for (const g of groups.filter((x) => x.chats.length === 0)) {
    if (groups.length === 1) break;
    const order = groupsOf(layout);
    const i = order.indexOf(g.id);
    layout = removeFromLayout(layout, g.id)!;
    groups = groups.filter((x) => x.id !== g.id);
    // The focus moves to the closed group's neighbor, like closing an editor split.
    if (focus === g.id) focus = order[i - 1] ?? order[i + 1]!;
  }
  return { ...s, groups, layout, focus };
}

/** Moves `tab` (from whichever group has it) into a new group on `side` of group `target`,
 * focused. A group left empty closes, so splitting a group's only tab moves the group. */
export function splitTab(s: State, target: string, tab: string, side: SplitSide, id: string): State {
  const added = { ...s, groups: [...s.groups, { id, chats: [tab], chat: tab }], layout: splitLayout(s.layout, target, id, side) };
  return { ...dropTabs(added, (t) => t === tab, id), focus: id };
}

/** Moves `tab` into group `to` at `index` (counted without the tab itself), active and focused. */
export function moveTab(s: State, tab: string, to: string, index: number): State {
  const groups = s.groups.map((g) => {
    if (g.id !== to) return g;
    const chats = g.chats.filter((t) => t !== tab);
    chats.splice(Math.max(0, Math.min(index, chats.length)), 0, tab);
    return { ...g, chats, chat: tab };
  });
  return { ...dropTabs({ ...s, groups }, (t) => t === tab, to), focus: to };
}

export interface Tabs {
  groups: Group[];
  layout: Layout;
  focus: string;
  /** Every open chat/file tab, across groups. */
  chats: string[];
  /** The focused group's active tab. */
  chat: string | null;
  boards: string[];
  /** Shows a tab: where it's already open, else in the focused group. */
  openChat: (tab: string) => void;
  /** Closes a chat tab; the neighbor becomes active. Returns the new active tab. */
  closeChat: (tab: string) => string | null;
  /** Closes several chat tabs at once (tab context menu). Returns the new active tab. */
  closeChats: (tabs: string[]) => string | null;
  /** A draft became a real chat on its first message. */
  replaceChat: (from: string, to: string) => void;
  focusGroup: (id: string) => void;
  /** Moves a tab into a new group beside group `target`. */
  split: (target: string, tab: string, side: SplitSide) => void;
  /** Moves a tab into a group at a position (dragged onto a tab bar). */
  move: (tab: string, to: string, index: number) => void;
  openBoard: (projectId: string) => void;
  closeBoard: (projectId: string) => void;
  /** Drops tabs of chats/projects that no longer exist. */
  prune: (keepChat: (tab: string) => boolean, keepProject: (id: string) => boolean) => void;
}

export function useTabs(): Tabs {
  const [s, setS] = useState<State>(() => loadState(readJsonPref("tabs", null, (_): _ is unknown => true)));
  useEffect(() => writePref("tabs", JSON.stringify(s)), [s]);

  const openChat = useCallback(
    (tab: string) =>
      setS((p) => {
        const home = p.groups.find((g) => g.chats.includes(tab)) ?? focused(p);
        const groups = p.groups.map((g) => (g.id === home.id ? { ...g, chat: tab, chats: g.chats.includes(tab) ? g.chats : [...g.chats, tab] } : g));
        return { ...p, groups, focus: home.id };
      }),
    [],
  );
  const closeChats = useCallback(
    (tabs: string[]) => {
      const next = dropTabs(s, (t) => tabs.includes(t));
      setS(next);
      return focused(next).chat;
    },
    [s],
  );
  const closeChat = useCallback((tab: string) => closeChats([tab]), [closeChats]);
  const replaceChat = useCallback(
    (from: string, to: string) =>
      setS((p) => ({
        ...p,
        groups: p.groups.map((g) => ({
          ...g,
          chats: g.chats.includes(to) ? g.chats.filter((t) => t !== from) : g.chats.map((t) => (t === from ? to : t)),
          chat: g.chat === from ? to : g.chat,
        })),
      })),
    [],
  );
  const focusGroup = useCallback((id: string) => setS((p) => (p.focus === id || !p.groups.some((g) => g.id === id) ? p : { ...p, focus: id })), []);
  const split = useCallback(
    (target: string, tab: string, side: SplitSide) => setS((p) => splitTab(p, target, tab, side, `g${crypto.randomUUID().slice(0, 8)}`)),
    [],
  );
  const move = useCallback((tab: string, to: string, index: number) => setS((p) => moveTab(p, tab, to, index)), []);
  const openBoard = useCallback(
    (id: string) => setS((p) => (p.boards.includes(id) ? p : { ...p, boards: [...p.boards, id] })),
    [],
  );
  const closeBoard = useCallback((id: string) => setS((p) => ({ ...p, boards: p.boards.filter((b) => b !== id) })), []);
  const prune = useCallback(
    (keepChat: (tab: string) => boolean, keepProject: (id: string) => boolean) =>
      setS((p) => {
        const gone = p.groups.some((g) => !g.chats.every(keepChat));
        const boards = p.boards.filter(keepProject);
        if (!gone && boards.length === p.boards.length) return p;
        return { ...dropTabs(p, (t) => !keepChat(t)), boards };
      }),
    [],
  );

  const chats = useMemo(() => s.groups.flatMap((g) => g.chats), [s.groups]);
  return { ...s, chats, chat: focused(s).chat, openChat, closeChat, closeChats, replaceChat, focusGroup, split, move, openBoard, closeBoard, prune };
}
