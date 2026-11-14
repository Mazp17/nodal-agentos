import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Archive, ArchiveRestore, GitBranch, Link, Pencil, Trash2, Unlink } from "lucide-react";
import { DropdownMenu } from "radix-ui";
import { relUnder } from "../../domain/hooks/projects";
import {
  archiveChat,
  cleanupWorktree,
  createChat,
  interruptChat,
  linkChatTask,
  rewindChat,
  sendChatMessage,
  unarchiveChat,
  updateChat,
  updateTask,
  type ChatPatch,
} from "../../domain/api";
import { useChatWorkspace } from "../../domain/hooks/branches";
import { useChatAgents, useChats, useClaudeDefaults } from "../../domain/hooks/chats";
import { hiddenSetOf, invalidate, isExecutorHidden, setData, useHiddenExecutors, useTask, useTasks } from "../../domain/hooks/store";
import type { Chat, Project, Repo } from "../../domain/types";
import { ExternalLink } from "../../ui/ExternalLink";
import { useConfirm } from "../../ui/ConfirmDialog";
import { EmptyState } from "../../ui/EmptyState";
import { Tooltip, TooltipContent, TooltipTrigger } from "../../ui/Tooltip";
import { useToast } from "../../ui/Toasts";
import { CommandPalette } from "../../shell/palette/CommandPalette";
import { isClosed, providerLabel, STATUS_META, taskTag } from "../tasks/status";
import { PrBadge, StatusRing } from "../tasks/bits";
import { Composer, type ComposerSettings } from "./Composer";
import { getComposerDraft, setComposerDraft, useComposerDraft } from "./composerDrafts";
import { Conversation } from "./Conversation";
import { ShellStrip } from "./ShellStrip";
import { closeSubagent, SubagentSheet, useSubagentTarget } from "./SubagentSheet";
import { agentOptions, chatTitle, effortOptions, modelOptions, modeOptions, repoOptions, toTurns } from "./model";
import {
  dropOutbox,
  ensureChatAttached,
  markInterrupting,
  pushOutbox,
  seedChat,
  setViewedChat,
  truncateAt,
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
  /** Asks, then deletes the chat (the sidebar's own flow). */
  onDeleteChat: (chat: Chat) => void;
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
export function ChatView({ project, repos, chatId: tabChatId, onCreated, onShowChanges, onOpenTask, onOpenDiff, onDeleteChat }: ChatViewProps) {
  const toast = useToast();
  const ask = useConfirm();
  const chats = useChats(project.id);
  const draftKey = tabChatId ?? `new:${project.id}`;
  const draft = useComposerDraft(draftKey);
  const setDraft = (text: string) => setComposerDraft(draftKey, text);
  const [menu, setMenu] = useState<string | null>(null);
  const [draftSettings, setDraftSettings] = useState<ComposerSettings>(NEW_CHAT);

  const chat = useMemo(() => (tabChatId ? ((chats.data ?? []).find((c) => c.id === tabChatId) ?? null) : null), [chats.data, tabChatId]);
  const chatId = chat?.id ?? null;
  const task = useTask(chat?.taskId ?? null).data;
  const stream = useChatStream(chatId);
  const subagent = useSubagentTarget(chatId);
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
      // The text goes to the new chat: the next new chat starts empty.
      setComposerDraft(draftKey, "");
      onCreated(target.id);
    }
    const id = target.id;
    pushOutbox(id, text);
    // By id, not `setDraft`: a new chat's tab is re-keyed to `id` (and remounted) meanwhile.
    setComposerDraft(id, "");
    atBottom.current = true;
    try {
      // Its channel must be attached before the process can start (and stream): `seedChat`/
      // `useChatStream` already kicked this off, cached and idempotent, so this just awaits it.
      await ensureChatAttached(id);
      await sendChatMessage(id, text);
      void invalidate("chats");
    } catch (e) {
      dropOutbox(id, text);
      setComposerDraft(id, getComposerDraft(id) || text);
      toast("Couldn't send the message", String(e), "danger");
    }
  };

  // ---- Header bar actions: rename, archive, link to a task, delete.
  const [renaming, setRenaming] = useState(false);
  const [picking, setPicking] = useState(false);
  const projectTasks = useTasks(picking ? project.id : null).data;
  const act = (what: string, run: () => Promise<unknown>) =>
    run().then(
      () => void invalidate("chats", "tasks"),
      (e: unknown) => toast(`Couldn't ${what}`, String(e), "danger"),
    );
  const rename = (title: string) => {
    setRenaming(false);
    if (chat && title.trim() && title.trim() !== chatTitle(chat)) void act("rename the chat", () => updateChat(chat.id, { title: title.trim() }));
  };
  const linkable = useMemo(() => {
    const taken = new Set((chats.data ?? []).map((c) => c.taskId));
    return (projectTasks ?? []).filter((t) => !isClosed(t.status) && !taken.has(t.id));
  }, [projectTasks, chats.data]);

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

  /**
   * Goes back to just before the message `uuid`: asks first when files Claude edited would be
   * restored (the user's own edits to them would go too), then `then()` (resend or refill).
   */
  const rewind = async (uuid: string, then: () => void) => {
    if (!chatId || busy) return;
    try {
      const dry = await rewindChat(chatId, uuid, true);
      if (dry.filesChanged.length > 0) {
        const names = dry.filesChanged.map((f) => f.split(/[\\/]/).pop()).slice(0, 5).join(", ");
        const more = dry.filesChanged.length > 5 ? ` and ${dry.filesChanged.length - 5} more` : "";
        const ok = await ask({
          title: "Go back to this message?",
          body: `The conversation after it is removed, and ${dry.filesChanged.length} ${dry.filesChanged.length === 1 ? "file Claude edited goes" : "files Claude edited go"} back to how they were (+${dry.insertions} −${dry.deletions}): ${names}${more}. Your own changes to those files are lost too.`,
          confirmLabel: "Go back",
        });
        if (!ok) return;
      }
      const done = await rewindChat(chatId, uuid, false);
      truncateAt(chatId, uuid);
      if (done.filesError) toast("Files weren't restored", done.filesError, "warn");
      then();
    } catch (e) {
      toast("Couldn't go back to that message", String(e), "danger");
    }
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
        {chat && (
          <div className="chat-task">
            {renaming ? (
              <input
                className="chat-task-rename"
                aria-label="Chat title"
                defaultValue={chatTitle(chat)}
                autoFocus
                onBlur={(e) => rename(e.currentTarget.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && !e.nativeEvent.isComposing) e.currentTarget.blur();
                  else if (e.key === "Escape") {
                    e.stopPropagation();
                    setRenaming(false);
                  }
                }}
              />
            ) : task ? (
              <>
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
                {task.pr && <PrBadge pr={task.pr} link />}
                <span className="chat-task-status">
                  <StatusRing status={task.status} size={12} />
                  {STATUS_META[task.status].label}
                </span>
              </>
            ) : (
              <span className="chat-task-title chat-task-name ellipsis" title={chatTitle(chat)}>
                {chatTitle(chat)}
              </span>
            )}
            <span className="chat-task-actions">
              {chat.archivedAt ? (
                <button type="button" className="btn btn-xs btn-ghost" onClick={() => void act("unarchive the chat", () => unarchiveChat(chat.id))}>
                  Archived · Unarchive
                </button>
              ) : (
                !chat.taskId && (
                  <button type="button" className="btn btn-xs btn-ghost" onClick={() => setPicking(true)}>
                    Link to task
                  </button>
                )
              )}
              <DropdownMenu.Root>
                <DropdownMenu.Trigger className="icon-btn chat-task-more" aria-label="Chat actions" title="Chat actions">
                  ⋯
                </DropdownMenu.Trigger>
                <DropdownMenu.Portal>
                  <DropdownMenu.Content className="menu tab-menu" align="end" sideOffset={4} collisionPadding={8}>
                    <DropdownMenu.Item className="menu-item" onSelect={() => setRenaming(true)}>
                      <Pencil size={14} aria-hidden />
                      Rename
                    </DropdownMenu.Item>
                    {chat.archivedAt ? (
                      <DropdownMenu.Item className="menu-item" onSelect={() => void act("unarchive the chat", () => unarchiveChat(chat.id))}>
                        <ArchiveRestore size={14} aria-hidden />
                        Unarchive
                      </DropdownMenu.Item>
                    ) : (
                      <DropdownMenu.Item className="menu-item" onSelect={() => void act("archive the chat", () => archiveChat(chat.id))}>
                        <Archive size={14} aria-hidden />
                        Archive
                      </DropdownMenu.Item>
                    )}
                    {chat.taskId ? (
                      <DropdownMenu.Item className="menu-item" onSelect={() => void act("unlink the chat", () => linkChatTask(chat.id, null))}>
                        <Unlink size={14} aria-hidden />
                        Unlink from task
                      </DropdownMenu.Item>
                    ) : (
                      <DropdownMenu.Item className="menu-item" onSelect={() => setPicking(true)}>
                        <Link size={14} aria-hidden />
                        Link to task…
                      </DropdownMenu.Item>
                    )}
                    <DropdownMenu.Separator className="menu-sep" />
                    <DropdownMenu.Item className="menu-item menu-item-danger" onSelect={() => onDeleteChat(chat)}>
                      <Trash2 size={14} aria-hidden />
                      Delete
                    </DropdownMenu.Item>
                  </DropdownMenu.Content>
                </DropdownMenu.Portal>
              </DropdownMenu.Root>
            </span>
          </div>
        )}
        {picking && chat && (
          <CommandPalette
            actions={[]}
            placeholder="Link this chat to a task…"
            onClose={() => setPicking(false)}
            search={(q) =>
              linkable
                .filter((t) => !q || `${taskTag(t, project.key)} ${t.title}`.toLowerCase().includes(q))
                .map((t) => ({
                  id: t.id,
                  kind: "Task" as const,
                  label: `${taskTag(t, project.key)} ${t.title}`,
                  sub: STATUS_META[t.status].label,
                  run: () => void act("link the chat", () => linkChatTask(chat.id, t.id)),
                }))
            }
          />
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
                onRewind={(uuid, text) => void rewind(uuid, () => setDraft(text))}
                onResend={(uuid, text) => void rewind(uuid, () => void send(text))}
              />
            )}
          </div>
        )}
        {chatId && stream && <ShellStrip chatId={chatId} shells={stream.shells} onStopTurn={stop} />}
        <Composer
          tray={
            (workspace && workspace.files > 0) || canLand ? (
              <>
                <GitBranch size={13} aria-hidden className="chat-changes-icon" />
                <Tooltip>
                  <TooltipTrigger asChild>
                    {/* Focusable so the full name and path also show from the keyboard. */}
                    <span className="chat-changes-where ellipsis" tabIndex={0}>
                      {workspace?.branch
                        ? `${workspace.isWorktree ? "worktree · " : "on branch · "}${task && workspace.isWorktree ? taskKeyLabel : workspace.branch}`
                        : `worktree · ${taskKeyLabel}`}
                    </span>
                  </TooltipTrigger>
                  <TooltipContent>
                    <div>{workspace?.branch ?? task?.worktree?.branch}</div>
                    {workspace?.cwd && <div className="mono">{workspace.cwd}</div>}
                  </TooltipContent>
                </Tooltip>
                {workspace && workspace.files > 0 && (
                  <>
                    <span className="chat-changes-sep" aria-hidden />
                    <span className="chat-changes-files">
                      {workspace.files} {workspace.files === 1 ? "file" : "files"} changed
                    </span>
                    <span className="chat-changes-add num">+{workspace.additions}</span>
                    <span className="chat-changes-del num">−{workspace.deletions}</span>
                  </>
                )}
                <span className="spacer" />
                {canLand && task?.worktree && (
                  <button type="button" className="chat-tray-btn" disabled={busy} onClick={() => void discard()}>
                    Discard
                  </button>
                )}
                {workspace && workspace.files > 0 && (
                  <button type="button" className="chat-tray-btn chat-tray-btn-strong" onClick={onShowChanges}>
                    Review changes
                  </button>
                )}
              </>
            ) : undefined
          }
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
      {subagent && stream && <SubagentSheet target={subagent} stream={stream} live={stream.state === "busy"} onClose={closeSubagent} />}
    </div>
  );
}
