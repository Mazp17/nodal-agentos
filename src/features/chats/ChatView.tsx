import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { relUnder } from "../../domain/hooks/projects";
import { cleanupWorktree, createChat, interruptChat, sendChatMessage, updateChat, updateTask, type ChatPatch } from "../../domain/api";
import { useChatWorkspace } from "../../domain/hooks/branches";
import { useChatAgents, useChats, useClaudeDefaults } from "../../domain/hooks/chats";
import { hiddenSetOf, invalidate, isExecutorHidden, setData, useHiddenExecutors, useTask } from "../../domain/hooks/store";
import type { Chat, Project, Repo } from "../../domain/types";
import { ExternalLink } from "../../ui/ExternalLink";
import { useConfirm } from "../../ui/ConfirmDialog";
import { EmptyState } from "../../ui/EmptyState";
import { useToast } from "../../ui/Toasts";
import { isClosed, providerLabel, STATUS_META, taskTag } from "../tasks/status";
import { StatusRing } from "../tasks/bits";
import { Composer, type ComposerSettings } from "./Composer";
import { Conversation } from "./Conversation";
import { agentOptions, chatTitle, effortOptions, modelOptions, modeOptions, repoOptions, toTurns } from "./model";
import {
  dropOutbox,
  ensureChatAttached,
  markInterrupting,
  pushOutbox,
  seedChat,
  setViewedChat,
  useChatStream,
} from "./stream";
import "./transcript.css";
import "./chat.css";

export interface ChatViewProps {
  project: Project;
  repos: Repo[];
  /** The chat this tab shows; `null` is a new chat, created on its first message. */
  chatId: string | null;
  /** A new chat was created: its tab now points at `id`. */
  onCreated: (id: string) => void;
  /** "N files changed" was clicked: show the uncommitted changes (files panel, git view). */
  onShowChanges: () => void;
  onOpenTask: (taskId: string) => void;
  /** Opens a file's changes: `worktree` is the checkout it lives in (`null`: the repo's own). */
  onOpenDiff: (repoId: string, path: string, worktree: string | null) => void;
}

const NEW_CHAT: ComposerSettings = { permissionMode: "auto", repoId: null, agent: null, model: null, effort: null };

const settingsOf = (c: Chat): ComposerSettings => ({
  permissionMode: c.permissionMode ?? null,
  repoId: c.repoId,
  agent: c.agent ?? null,
  model: c.model ?? null,
  effort: c.effort ?? null,
});

/** One chat tab: the conversation and the composer. */
export function ChatView({ project, repos, chatId: tabChatId, onCreated, onShowChanges, onOpenTask, onOpenDiff }: ChatViewProps) {
  const toast = useToast();
  const ask = useConfirm();
  const chats = useChats(project.id);
  const [draft, setDraft] = useState("");
  const [menu, setMenu] = useState<string | null>(null);
  const [draftSettings, setDraftSettings] = useState<ComposerSettings>(NEW_CHAT);

  const chat = useMemo(() => (tabChatId ? ((chats.data ?? []).find((c) => c.id === tabChatId) ?? null) : null), [chats.data, tabChatId]);
  const chatId = chat?.id ?? null;
  const task = useTask(chat?.taskId ?? null).data;
  const stream = useChatStream(chatId);
  useEffect(() => {
    setViewedChat(chatId);
    return () => setViewedChat(null);
  }, [chatId]);

  const settings = chat ? settingsOf(chat) : draftSettings;
  // A chat without a repo runs in the project's first one, so its settings apply.
  const defaults = useClaudeDefaults(settings.repoId ?? repos[0]?.id ?? null).data;
  // Agents of the folder the chat runs in, minus the ones the project hides from its pickers.
  const chatAgents = useChatAgents(project.id, settings.repoId);
  const folderAgents = chatAgents.data;
  const hiddenKeys = useHiddenExecutors(project.id).data;
  const agents = useMemo(() => {
    const hidden = hiddenSetOf(hiddenKeys);
    // Repo-level entries are keyed by the repo the chat runs in (none at the project root).
    const runsIn = settings.repoId ?? (project.rootPath ? null : (repos[0]?.id ?? null));
    // The chat's own agent stays listed even if the project hides it.
    const shown = (folderAgents ?? []).filter(
      (a) => (a.executor.kind === "agent" && a.executor.name === settings.agent) || !isExecutorHidden(hidden, a, runsIn),
    );
    return agentOptions(shown, settings.agent);
  }, [folderAgents, hiddenKeys, settings.repoId, settings.agent, project.rootPath, repos]);
  const busy = !!stream && (stream.state === "busy" || stream.outbox.length > 0);
  const workspace = useChatWorkspace(chatId).data ?? null;
  // An answer may have edited files or moved into a worktree: re-read when it ends.
  const wasBusy = useRef(false);
  useEffect(() => {
    if (wasBusy.current && !busy) void invalidate("chats");
    wasBusy.current = busy;
  }, [busy]);
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

  // Claude Code keeps a session's agent on `--resume`, so it's fixed once the chat has a
  // session (or while its first message is on the way). A first send that failed before one
  // was saved (e.g. an agent that doesn't exist) can still change it.
  const agentLocked = !!chat?.sessionId || busy;

  const setSetting = (key: keyof ComposerSettings, value: string | null) => {
    // An agent from the old repo's `.claude/agents` isn't there in the new one.
    const dropAgent =
      key === "repoId" &&
      !agentLocked &&
      !!folderAgents?.some((a) => a.source === "repo" && a.executor.kind === "agent" && a.executor.name === settings.agent);
    if (!chat) {
      setDraftSettings((s) => ({ ...s, [key]: value, ...(dropAgent ? { agent: null } : {}) }));
      return;
    }
    const patch: ChatPatch = { [key]: value, ...(dropAgent ? { agent: null } : {}) };
    const optimistic: Partial<Chat> = key === "repoId" || key === "agent" ? { [key]: value } : { [key]: value ?? undefined };
    if (dropAgent) optimistic.agent = null;
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
          agent: settings.agent,
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
      // The sidebar reads every project's chats: show the new one there (and keep its tab) right away.
      setData<Chat[]>("chats:*", (cs) => [target as Chat, ...cs]);
      void invalidate("chats");
      onCreated(target.id);
    }
    const id = target.id;
    pushOutbox(id, text);
    setDraft("");
    atBottom.current = true;
    try {
      // Its channel must be attached before the process can start (and stream): `seedChat`/
      // `useChatStream` already kicked this off, cached and idempotent, so this just awaits it.
      await ensureChatAttached(id);
      await sendChatMessage(id, text);
      void invalidate("chats");
    } catch (e) {
      dropOutbox(id, text);
      setDraft((d) => d || text);
      toast("Couldn't send the message", String(e), "danger");
    }
  };

  const taskKeyLabel = task ? taskTag(task, project.key) : "";
  // Discard acts on the task worktree, so it only makes sense while it exists and the task is open.
  const canLand = !!task?.worktree && !isClosed(task.status);

  const discard = async () => {
    if (!task?.worktree) return;
    const ok = await ask({
      title: `Discard ${taskKeyLabel}'s work?`,
      body: `Deletes the worktree and branch ${task.worktree.branch}, with any commits and changes in it. The task goes back to Todo.`,
      confirmLabel: "Discard work",
    });
    if (!ok) return;
    try {
      await cleanupWorktree(task.id, true);
      await updateTask(task.id, { status: "todo" });
      toast("Work discarded", `${taskKeyLabel} is back in Todo`, "ok");
    } catch (e) {
      toast("Couldn't discard the work", String(e), "danger");
    } finally {
      void invalidate("tasks", "chats");
    }
  };

  /** A path a tool edited (absolute, or relative to the chat's folder) → repo, checkout, relative path. */
  const openDiff = (p: string) => {
    const cwd = workspace?.cwd ?? null;
    const abs = /^([A-Za-z]:)?[\\/]/.test(p) ? p : cwd ? `${cwd}/${p}` : null;
    if (!abs) return;
    const under = (root: string) => relUnder(abs, root) !== null;
    // shortcut: takes the chat's folder as the worktree root, wrong if Claude cd'd into a subfolder of it.
    if (workspace?.isWorktree && cwd && under(cwd)) {
      const repo = repos.find((r) => relUnder(cwd, r.path) !== null) ?? repos.find((r) => r.id === chat?.repoId);
      if (repo) return onOpenDiff(repo.id, relUnder(abs, cwd)!, cwd);
    }
    const repo = repos.filter((r) => under(r.path)).sort((a, b) => b.path.length - a.path.length)[0];
    if (repo) onOpenDiff(repo.id, relUnder(abs, repo.path)!, null);
  };

  const stop = () => {
    if (!chatId) return;
    markInterrupting(chatId, true);
    interruptChat(chatId).catch((e: unknown) => {
      markInterrupting(chatId, false);
      toast("Couldn't stop the answer", String(e), "danger");
    });
  };

  const firstRepo = repos[0]?.name;
  const scopeRepo = repos.find((r) => r.id === settings.repoId)?.name;
  const suggestions = [
    scopeRepo || firstRepo ? `How is ${scopeRepo ?? firstRepo} structured?` : "How is this project structured?",
    "What's in progress on the board?",
    "Turn an idea into a task for the board",
  ];

  return (
    <div className="chat-page" data-screen-label="Chat">
      <section className="chat-main" aria-label={chat ? chatTitle(chat) : "New chat"}>
        {task && (
          <div className="chat-task">
            <span className="chat-task-tag mono">
              {task.source ? (
                <ExternalLink url={task.source.url} label={`Open ${taskKeyLabel} in ${providerLabel(task.source.provider)}`}>
                  {taskKeyLabel} ↗
                </ExternalLink>
              ) : (
                <button type="button" className="chat-task-link" onClick={() => onOpenTask(task.id)}>
                  {taskKeyLabel}
                </button>
              )}
            </span>
            <button type="button" className="chat-task-title ellipsis" title="Open the task" onClick={() => onOpenTask(task.id)}>
              {task.title}
            </button>
            <span className="chat-task-status">
              <StatusRing status={task.status} size={12} />
              {STATUS_META[task.status].label}
            </span>
          </div>
        )}
        {!hasMessages ? (
          chat && stream && !stream.loaded ? (
            <div className="chat-empty">
              <p className="chat-empty-text">Loading conversation…</p>
            </div>
          ) : chat && stream?.loadError ? (
            <div className="chat-empty">
              <p className="chat-empty-text tr-error" role="alert">
                Couldn't load this conversation: {stream.loadError}
              </p>
            </div>
          ) : (
            <EmptyState
              title={`New chat in ${project.name}`}
              description="Ask about the code, plan a change, or turn the conversation into tasks on the board. Claude sees the whole project; pick a repo below to narrow it."
            >
              {suggestions.map((s) => (
                <button key={s} type="button" className="btn" onClick={() => void send(s)}>
                  {s}
                </button>
              ))}
            </EmptyState>
          )
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
                onOpenDiff={openDiff}
              />
            )}
          </div>
        )}
        {((workspace && workspace.files > 0) || canLand) && (
          <div className="chat-changes">
            {workspace && workspace.files > 0 ? (
              <button type="button" className="chat-changes-btn" title="Show the changes" onClick={onShowChanges}>
                <span className="chat-changes-files">
                  {workspace.files} {workspace.files === 1 ? "file" : "files"} changed
                </span>
                <span className="chat-changes-add num">+{workspace.additions}</span>
                <span className="chat-changes-del num">−{workspace.deletions}</span>
                {workspace.branch && (
                  <span className="chat-changes-where ellipsis" title={workspace.cwd ?? undefined}>
                    {workspace.isWorktree ? "worktree · " : "on "}
                    {task && workspace.isWorktree ? taskKeyLabel : workspace.branch}
                  </span>
                )}
              </button>
            ) : (
              <span className="chat-changes-where ellipsis" title={task?.worktree?.branch}>
                worktree · {taskKeyLabel}
              </span>
            )}
            {canLand && task?.worktree && (
              <span className="chat-changes-actions">
                <button type="button" className="btn btn-xs btn-ghost chat-discard" disabled={busy} onClick={() => void discard()}>
                  Discard
                </button>
              </span>
            )}
          </div>
        )}
        <Composer
          draft={draft}
          placeholder={`Ask anything about ${project.name}…`}
          settings={settings}
          modes={modeOptions(settings.permissionMode)}
          repoOptions={repoOptions(repos)}
          agents={agents}
          agentLocked={agentLocked}
          models={modelOptions(defaults?.model)}
          efforts={effortOptions(defaults?.effort)}
          menu={menu}
          busy={busy}
          contextTokens={stream?.contextTokens ?? null}
          contextWindow={stream?.contextWindow ?? null}
          focusKey={chatId ?? "new"}
          onDraft={setDraft}
          onMenu={(id) => {
            // Agent files can appear while the page is open: re-read them when the menu opens.
            if (id === "agent") void chatAgents.refresh();
            setMenu(id);
          }}
          onSetting={setSetting}
          onSend={() => void send()}
          onStop={stop}
        />
      </section>
    </div>
  );
}
