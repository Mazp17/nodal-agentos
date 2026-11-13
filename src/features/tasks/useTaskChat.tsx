import { createContext, useCallback, useContext } from "react";
import { startTaskChat } from "../../domain/api";
import { invalidate, setData } from "../../domain/hooks/store";
import type { Chat } from "../../domain/types";
import { useToast } from "../../ui/Toasts";
import { sendToChat } from "../chats/stream";

/** The shell provides how to open a chat tab; everything that starts a task chat reuses it. */
export const OpenChatContext = createContext<((c: Chat) => void) | null>(null);

const withChat = (cs: Chat[], c: Chat) => (cs.some((x) => x.id === c.id) ? cs : [c, ...cs]);

/** Chats whose first message is on its way: a second click mustn't send it twice. */
const sending = new Set<string>();

/**
 * "Open in chat": creates (or reuses) the task's chat, then opens its tab. Resolves with the
 * chat, or `null` if the backend refused (the reason goes to a toast).
 */
export function useOpenTaskChat() {
  const open = useContext(OpenChatContext);
  const toast = useToast();
  return useCallback(
    async (taskId: string): Promise<Chat | null> => {
      try {
        const { chat, prompt } = await startTaskChat(taskId);
        // Show it right away: the tab looks the chat up in these lists.
        setData<Chat[]>(`chats:${chat.projectId}`, (cs) => withChat(cs, chat));
        setData<Chat[]>("chats:*", (cs) => withChat(cs, chat));
        void invalidate("chats", "tasks");
        if (prompt && !sending.has(chat.id)) {
          // Sent like a typed message (attached channel first), so it shows and streams at once.
          sending.add(chat.id);
          open?.(chat);
          try {
            await sendToChat(chat.id, prompt);
            void invalidate("chats");
          } catch (e) {
            toast("Couldn't send the task to the chat", String(e), "danger");
          } finally {
            sending.delete(chat.id);
          }
          return chat;
        }
        open?.(chat);
        return chat;
      } catch (e) {
        toast("Couldn't open the task in a chat", String(e), "danger");
        return null;
      }
    },
    [open, toast],
  );
}
