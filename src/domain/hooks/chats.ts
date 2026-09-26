// "chats" resource of the data layer (see `store.ts`): a project's chats, re-read on
// `nodal://changed` `chats`. What a chat streams lives in `features/chats/stream.ts`.

import { getClaudeDefaults, listChats, type ClaudeDefaults } from "../api";
import type { Chat } from "../types";
import { POLL, usePolled, type Loadable } from "./store";

/** Most recently used first. */
export const useChats = (projectId: string | null): Loadable<Chat[]> =>
  usePolled<Chat[]>(projectId ? `chats:${projectId}` : null, () => listChats(projectId), ["chats"], POLL.slow);

/** Every project's chats, most recently used first (for the command palette). */
export const useAllChats = (enabled: boolean): Loadable<Chat[]> =>
  usePolled<Chat[]>(enabled ? "chats:*" : null, () => listChats(null), ["chats"], POLL.slow);

/** Claude Code's configured model and effort for a repo (or only the user's settings). */
export const useClaudeDefaults = (repoId: string | null): Loadable<ClaudeDefaults> =>
  usePolled<ClaudeDefaults>(`claude-defaults:${repoId ?? "*"}`, () => getClaudeDefaults(repoId), [], POLL.slow);
