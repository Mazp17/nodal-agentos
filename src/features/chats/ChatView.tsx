import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createChat, deleteChat, interruptChat, sendChatMessage, updateChat, type ChatPatch } from "../../domain/api";
import { useChats } from "../../domain/hooks/chats";
import { invalidate, setData } from "../../domain/hooks/store";
import type { Chat, Project, Repo } from "../../domain/types";
import { readJsonPref, writePref } from "../../shell/storage";
import { BrandMark } from "../../ui/BrandMark";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { Composer, type ComposerSettings } from "./Composer";
import { Conversation } from "./Conversation";
import { EFFORTS, MODELS, MODES, repoOptions, shortAgo, toTurns } from "./model";
import { dropOutbox, forgetChat, markInterrupting, pushOutbox, seedChat, useChatStream, useRunStates } from "./stream";
import "../runs/transcript.css";
import "./chat.css";

export interface ChatViewProps {
  project: Project;
  repos: Repo[];
  onOpenTask: (taskId: string) => void;
}

/** Selected chat per project; `null` is "New chat". Not stored: the most recent one. */
const SELECTED_PREF = "chatSelected";
const isSelection = (v: unknown): v is Record<string, string | null> => !!v && typeof v === "object" && !Array.isArray(v);

const NEW_CHAT: ComposerSettings = { permissionMode: null, repoId: null, model: null, effort: null };

const settingsOf = (c: Chat): ComposerSettings => ({
  permissionMode: c.permissionMode ?? null,
  repoId: c.repoId,
  model: c.model ?? null,
  effort: c.effort ?? null,
});

/** The Chat page: the project's sessions, the selected conversation and the composer. */
export function ChatView({ project, repos, onOpenTask }: ChatViewProps) {
  const toast = useToast();
  const ask = useConfirm();
  const chats = useChats(project.id);
  const runStates = useRunStates();
  const [selection, setSelection] = useState(() => readJsonPref(SELECTED_PREF, {}, isSelection));
  const [q, setQ] = useState("");
  const [draft, setDraft] = useState("");
  const [menu, setMenu] = useState<string | null>(null);
  const [draftSettings, setDraftSettings] = useState<ComposerSettings>(NEW_CHAT);
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(t);
  }, []);
  useEffect(() => writePref(SELECTED_PREF, JSON.stringify(selection)), [selection]);

  const list = useMemo(() => chats.data ?? [], [chats.data]);
  const stored = selection[project.id];
  const chat = stored === null ? null : (list.find((c) => c.id === stored) ?? list[0] ?? null);
  const chatId = chat?.id ?? null;
  const stream = useChatStream(chatId);
  const select = (id: string | null) => {
    setSelection((s) => ({ ...s, [project.id]: id }));
    setMenu(null);
  };

  const settings = chat ? settingsOf(chat) : draftSettings;
  const busy = !!stream && (stream.state === "busy" || stream.outbox.length > 0);
  const turns = useMemo(() => (stream ? toTurns(stream.items, stream.outbox) : []), [stream]);
  const hasMessages = turns.length > 0 || !!stream?.streaming;

  // Follows the answer while the user is at the bottom.
  const scroller = useRef<HTMLDivElement>(null);
  const atBottom = useRef(true);
  useLayoutEffect(() => {
    atBottom.current = true;
  }, [chatId]);
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el && atBottom.current) el.scrollTop = el.scrollHeight;
  }, [stream, chatId]);

  const setSetting = (key: keyof ComposerSettings, value: string | null) => {
    if (!chat) {
      setDraftSettings((s) => ({ ...s, [key]: value }));
      return;
    }
    const patch: ChatPatch = { [key]: value };
    const optimistic: Partial<Chat> = key === "repoId" ? { repoId: value } : { [key]: value ?? undefined };
    setData<Chat[]>(`chats:${project.id}`, (cs) => cs.map((c) => (c.id === chat.id ? { ...c, ...optimistic } : c)));
    updateChat(chat.id, patch).then(
      () => void invalidate("chats"),
      (e: unknown) => {
        void invalidate("chats");
        toast("Couldn't change the chat", String(e), "danger");
      },
    );
  };

  const send = async (raw?: string) => {
    const text = (raw ?? draft).trim();
    if (!text || busy) return;
    let target = chat;
    if (!target) {
      try {
        target = await createChat(project.id, {
          repoId: settings.repoId,
          model: settings.model ?? undefined,
          effort: settings.effort ?? undefined,
          permissionMode: settings.permissionMode ?? undefined,
        });
      } catch (e) {
        toast("Couldn't start the chat", String(e), "danger");
        return;
      }
      seedChat(target.id);
      setData<Chat[]>(`chats:${project.id}`, (cs) => [target as Chat, ...cs]);
      select(target.id);
    }
    const id = target.id;
    pushOutbox(id, text);
    setDraft("");
    atBottom.current = true;
    try {
      await sendChatMessage(id, text);
      void invalidate("chats");
    } catch (e) {
      dropOutbox(id, text);
      setDraft((d) => d || text);
      toast("Couldn't send the message", String(e), "danger");
    }
  };

  const stop = () => {
    if (!chatId) return;
    markInterrupting(chatId, true);
    interruptChat(chatId).catch((e: unknown) => {
      markInterrupting(chatId, false);
      toast("Couldn't stop the answer", String(e), "danger");
    });
  };

  const remove = async (c: Chat) => {
    const ok = await ask({
      title: `Delete “${c.title ?? "New chat"}”?`,
      body: "It leaves the sessions list. The Claude Code session stays on disk and can still be resumed with claude --resume.",
      confirmLabel: "Delete chat",
    });
    if (!ok) return;
    try {
      await deleteChat(c.id);
      forgetChat(c.id);
      if (chatId === c.id) select(list.find((x) => x.id !== c.id)?.id ?? null);
      void invalidate("chats");
    } catch (e) {
      toast("Couldn't delete the chat", String(e), "danger");
    }
  };

  const ql = q.trim().toLowerCase();
  const sessions = list.filter((c) => !ql || (c.title ?? "New chat").toLowerCase().includes(ql));
  const firstRepo = repos[0]?.name;
  const scopeRepo = repos.find((r) => r.id === settings.repoId)?.name;
  const suggestions = [
    scopeRepo || firstRepo ? `How is ${scopeRepo ?? firstRepo} structured?` : "How is this project structured?",
    "What's in progress on the board?",
    "Turn an idea into a task for the board",
  ];

  return (
    <div className="chat-page" data-screen-label="Chat">
      <aside className="chat-side" aria-label="Chats">
        <button type="button" className="btn chat-new" onClick={() => select(null)}>
          <span className="chat-new-plus" aria-hidden>
            +
          </span>
          New chat
        </button>
        <input className="input chat-search" value={q} placeholder="Search chats" aria-label="Search chats" onChange={(e) => setQ(e.target.value)} />
        <h2 className="chat-side-label">Sessions</h2>
        <nav className="chat-sessions" aria-label="Sessions">
          {sessions.map((c) => {
            const on = c.id === chatId;
            const live = runStates.get(c.id) === "busy";
            return (
              <div key={c.id} className={`chat-session ${on ? "on" : ""}`}>
                <button
                  type="button"
                  className="chat-session-pick"
                  aria-current={on ? "page" : undefined}
                  title={c.sessionId ? `claude --resume ${c.sessionId}` : undefined}
                  onClick={() => select(c.id)}
                >
                  <span className="ellipsis chat-session-title">{c.title ?? "New chat"}</span>
                  {live && <span className="dot dot-sm pulse tone-accent" aria-label="Answering" />}
                  <span className="chat-session-ago num">{shortAgo(c.updatedAt, now)}</span>
                </button>
                <button type="button" className="icon-btn chat-session-del" aria-label={`Delete ${c.title ?? "New chat"}`} title="Delete chat" onClick={() => void remove(c)}>
                  ✕
                </button>
              </div>
            );
          })}
          {chats.error && !chats.data && <span className="chat-side-empty">Couldn't load chats: {chats.error}</span>}
          {chats.data && sessions.length === 0 && <span className="chat-side-empty">{ql ? "No matches" : "No chats yet"}</span>}
        </nav>
      </aside>

      <section className="chat-main" aria-label={chat?.title ?? "New chat"}>
        {!hasMessages ? (
          <div className="chat-empty">
            {chat && stream && !stream.loaded ? (
              <p className="chat-empty-text">Loading conversation…</p>
            ) : chat && stream?.loadError ? (
              <p className="chat-empty-text tr-error" role="alert">
                Couldn't load this conversation: {stream.loadError}
              </p>
            ) : (
              <div className="chat-empty-body">
                <BrandMark size={44} />
                <h2 className="chat-empty-title">New chat in {project.name}</h2>
                <p className="chat-empty-text">
                  Ask about the code, plan a change, or turn the conversation into tasks on the board. Claude sees the whole
                  project; pick a repo below to narrow it.
                </p>
                <div className="chat-suggestions">
                  {suggestions.map((s) => (
                    <button key={s} type="button" className="chat-suggestion" onClick={() => void send(s)}>
                      {s}
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>
        ) : (
          <div
            ref={scroller}
            className="chat-scroll"
            aria-live="polite"
            onScroll={(e) => {
              const el = e.currentTarget;
              atBottom.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
            }}
          >
            {stream && chatId && (
              <Conversation
                chatId={chatId}
                turns={turns}
                stream={stream}
                projectId={project.id}
                projectKey={project.key}
                repos={repos}
                onOpenTask={onOpenTask}
              />
            )}
          </div>
        )}
        <Composer
          draft={draft}
          placeholder={`Ask anything about ${project.name}…`}
          settings={settings}
          modes={MODES}
          repoOptions={repoOptions(repos)}
          models={MODELS}
          efforts={EFFORTS}
          menu={menu}
          busy={busy}
          contextTokens={stream?.contextTokens ?? null}
          contextWindow={stream?.contextWindow ?? null}
          focusKey={chatId ?? "new"}
          onDraft={setDraft}
          onMenu={setMenu}
          onSetting={setSetting}
          onSend={() => void send()}
          onStop={stop}
        />
      </section>
    </div>
  );
}
