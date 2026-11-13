// "chats" resource of the data layer (see `store.ts`): a project's chats, re-read on
// `nodal://changed` `chats`. What a chat streams lives in `features/chats/stream.ts`.

import { getClaudeDefaults, listChatAgents, listChatSubagents, listChats, type ClaudeDefaults, type ExecutorInfo } from "../api";
import type { SubagentRef } from "../../features/chats/types";
import type { Chat } from "../types";
import { POLL, usePolled, type Loadable } from "./store";

/** Most recently used first. */
export const useChats = (projectId: string | null): Loadable<Chat[]> =>
  usePolled<Chat[]>(projectId ? `chats:${projectId}` : null, () => listChats(projectId), ["chats"], POLL.slow);

/** Every project's chats, most recently used first (for the command palette). */
export const useAllChats = (enabled: boolean): Loadable<Chat[]> =>
  usePolled<Chat[]>(enabled ? "chats:*" : null, () => listChats(null), ["chats"], POLL.slow);

/** Agents in the folder a chat runs in; it reads disk, so only re-read on `repos`/`projects` notices. */
export const useChatAgents = (projectId: string, repoId: string | null): Loadable<ExecutorInfo[]> =>
  usePolled<ExecutorInfo[]>(`chat-agents:${projectId}:${repoId ?? "*"}`, () => listChatAgents(projectId, repoId), ["repos", "projects"], 0);

/** Claude Code's configured model and effort for a repo (or only the user's settings). */
export const useClaudeDefaults = (repoId: string | null): Loadable<ClaudeDefaults> =>
  usePolled<ClaudeDefaults>(`claude-defaults:${repoId ?? "*"}`, () => getClaudeDefaults(repoId), [], POLL.slow);

/** The chat's subagents; the Agents tab re-reads it when it opens and when an `Agent` call finishes. */
export const useChatSubagents = (chatId: string | null): Loadable<SubagentRef[]> =>
  usePolled<SubagentRef[]>(chatId ? `chat-subagents:${chatId}` : null, () => listChatSubagents(chatId!), ["chats"], POLL.slow);
