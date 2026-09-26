// "chats" resource of the data layer (see `store.ts`): a project's chats, re-read on
// `nodal://changed` `chats`. What a chat streams lives in `features/chats/stream.ts`.

import { listChats } from "../api";
import type { Chat } from "../types";
import { POLL, usePolled, type Loadable } from "./store";

/** Most recently used first. */
export const useChats = (projectId: string | null): Loadable<Chat[]> =>
  usePolled<Chat[]>(projectId ? `chats:${projectId}` : null, () => listChats(projectId as string), ["chats"], POLL.slow);
