// Open tabs: chats (across projects) and boards. Persisted in `nodal.tabs` so the
// workspace comes back as it was.

import { useCallback, useEffect, useState } from "react";
import { readJsonPref, writePref } from "./storage";

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

interface State {
  chats: string[];
  /** Active chat tab. */
  chat: string | null;
  /** Project boards open next to the pinned "All projects". */
  boards: string[];
}

const EMPTY: State = { chats: [], chat: null, boards: [] };
const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");
const isState = (v: unknown): v is State => {
  if (!v || typeof v !== "object") return false;
  const s = v as Record<string, unknown>;
  return isStrings(s.chats) && isStrings(s.boards) && (s.chat === null || typeof s.chat === "string");
};

export interface Tabs extends State {
  openChat: (tab: string) => void;
  /** Closes a chat tab; the neighbor becomes active. Returns the new active tab. */
  closeChat: (tab: string) => string | null;
  /** Closes several chat tabs at once (tab context menu). Returns the new active tab. */
  closeChats: (tabs: string[]) => string | null;
  /** A draft became a real chat on its first message. */
  replaceChat: (from: string, to: string) => void;
  openBoard: (projectId: string) => void;
  closeBoard: (projectId: string) => void;
  /** Drops tabs of chats/projects that no longer exist. */
  prune: (keepChat: (tab: string) => boolean, keepProject: (id: string) => boolean) => void;
}

export function useTabs(): Tabs {
  const [s, setS] = useState<State>(() => readJsonPref("tabs", EMPTY, isState));
  useEffect(() => writePref("tabs", JSON.stringify(s)), [s]);

  const openChat = useCallback(
    (tab: string) => setS((p) => ({ ...p, chat: tab, chats: p.chats.includes(tab) ? p.chats : [...p.chats, tab] })),
    [],
  );
  const closeChats = useCallback(
    (tabs: string[]) => {
      const drop = new Set(tabs);
      const chats = s.chats.filter((t) => !drop.has(t));
      let chat = s.chat;
      if (chat && drop.has(chat)) {
        // The nearest tab left of the closed active one, else the first that stays.
        const i = s.chats.indexOf(chat);
        chat = [...s.chats.slice(0, i)].reverse().find((t) => !drop.has(t)) ?? chats[0] ?? null;
      }
      setS({ ...s, chats, chat });
      return chat;
    },
    [s],
  );
  const closeChat = useCallback((tab: string) => closeChats([tab]), [closeChats]);
  const replaceChat = useCallback(
    (from: string, to: string) =>
      setS((p) => ({
        ...p,
        chats: p.chats.includes(to) ? p.chats.filter((t) => t !== from) : p.chats.map((t) => (t === from ? to : t)),
        chat: p.chat === from ? to : p.chat,
      })),
    [],
  );
  const openBoard = useCallback(
    (id: string) => setS((p) => (p.boards.includes(id) ? p : { ...p, boards: [...p.boards, id] })),
    [],
  );
  const closeBoard = useCallback((id: string) => setS((p) => ({ ...p, boards: p.boards.filter((b) => b !== id) })), []);
  const prune = useCallback(
    (keepChat: (tab: string) => boolean, keepProject: (id: string) => boolean) =>
      setS((p) => {
        const chats = p.chats.filter(keepChat);
        const boards = p.boards.filter(keepProject);
        if (chats.length === p.chats.length && boards.length === p.boards.length) return p;
        const chat = p.chat && chats.includes(p.chat) ? p.chat : (chats[chats.length - 1] ?? null);
        return { chats, chat, boards };
      }),
    [],
  );

  return { ...s, openChat, closeChat, closeChats, replaceChat, openBoard, closeBoard, prune };
}
