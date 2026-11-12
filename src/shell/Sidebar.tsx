import { useEffect, useState } from "react";
import { useChatWorkspace, useRepoBranch } from "../domain/hooks/branches";
import type { Chat, Project } from "../domain/types";
import { chatTitle, shortAgo } from "../features/chats/model";
import { BrandMark } from "../ui/BrandMark";
import { Kbd, KbdGroup } from "../ui/Kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/Tooltip";
import { readJsonPref, writePref } from "./storage";

export type SideMode = "chat" | "tasks";
type IconName = "chat" | "board" | "settings" | "plus" | "chevron" | "branch" | "trash";

const GEAR =
  "M5.9 1.5h2.2l.35 1.55c.4.15.77.36 1.1.62l1.52-.48 1.1 1.9-1.17 1.08a4 4 0 0 1 0 1.26l1.17 1.08-1.1 1.9-1.52-.48c-.33.26-.7.47-1.1.62L8.1 12.5H5.9l-.35-1.55a4 4 0 0 1-1.1-.62l-1.52.48-1.1-1.9 1.17-1.08a4 4 0 0 1 0-1.26L1.83 5.49l1.1-1.9 1.52.48c.33-.26.7-.47 1.1-.62z";

/** 14px line icons from the Nodal design. */
function SideIcon({ name, size = 14 }: { name: IconName; size?: number }) {
  return (
    <svg
      className="side-icon"
      width={size}
      height={size}
      viewBox="0 0 14 14"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.3"
      strokeLinejoin="round"
      strokeLinecap="round"
      aria-hidden
      focusable="false"
    >
      {name === "board" && (
        <>
          <rect x="1.5" y="2" width="3" height="10" rx="1" />
          <rect x="5.5" y="2" width="3" height="7" rx="1" />
          <rect x="9.5" y="2" width="3" height="4.5" rx="1" />
        </>
      )}
      {name === "chat" && (
        <path d="M3.5 2h7A1.5 1.5 0 0 1 12 3.5v5A1.5 1.5 0 0 1 10.5 10H6.5L4 12v-2h-.5A1.5 1.5 0 0 1 2 8.5v-5A1.5 1.5 0 0 1 3.5 2z" />
      )}
      {name === "settings" && (
        <>
          <path d={GEAR} />
          <circle cx="7" cy="7" r="1.8" />
        </>
      )}
      {name === "plus" && <path d="M7 3v8M3 7h8" strokeWidth="1.5" />}
      {name === "chevron" && <path d="M5.5 3.5l3.5 3.5-3.5 3.5" strokeWidth="1.4" />}
      {name === "branch" && (
        <>
          <circle cx="4" cy="3" r="1.4" />
          <circle cx="4" cy="11" r="1.4" />
          <circle cx="10" cy="4.5" r="1.4" />
          <path d="M4 4.4v5.2M10 5.9c0 2.4-6 1.8-6 3.4" />
        </>
      )}
      {name === "trash" && <path d="M2.5 4h9M5.5 4V2.5h3V4M3.8 4l.6 7.5h5.2l.6-7.5" />}
    </svg>
  );
}

function Keys({ keys }: { keys: string[] }) {
  return (
    <KbdGroup aria-hidden>
      {keys.map((k) => (
        <Kbd key={k}>{k}</Kbd>
      ))}
    </KbdGroup>
  );
}

export interface ProviderFoot {
  tone: "ok" | "danger" | "muted";
  label: string;
}

/** Chats listed per project before "Show N more". */
const RECENT = 5;
const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");

interface Props {
  mode: SideMode;
  projects: Project[];
  /** Every project's chats, most recently used first. */
  chats: Chat[];
  /** The repo a chat without one runs in (the project's first). */
  defaultRepo: (projectId: string) => string | null;
  /** Active chat tab (a chat id, or a draft). */
  activeChat: string | null;
  liveChats: ReadonlySet<string>;
  unreadChats: ReadonlyMap<string, string>;
  /** Board showing: `null` is "All projects"; `undefined` when not on a board. */
  activeBoard: string | null | undefined;
  settingsOpen: boolean;
  openTotal: number;
  openByProject: ReadonlyMap<string, number>;
  needYouByProject: ReadonlySet<string>;
  /** Projects with sources whose states changed in the provider (mapping to review). */
  mappingDrift: ReadonlySet<string>;
  canNewTask: boolean;
  provider: ProviderFoot;
  /** Installed version, and the newest one if the update check found one. */
  version: string | null;
  updateAvailable: string | null;
  updating: boolean;
  checkingUpdates: boolean;
  onUpdate: () => void;
  onMode: (mode: SideMode) => void;
  onOpenChat: (chat: Chat) => void;
  onNewChat: (projectId: string) => void;
  onDeleteChat: (chat: Chat) => void;
  onOpenBoard: (projectId: string | null) => void;
  onNewTask: () => void;
  onProjectSettings: (projectId: string) => void;
  onSettings: () => void;
  onNewProject: () => void;
  onOpenPalette: () => void;
}

export function Sidebar(p: Props) {
  const [collapsed, setCollapsed] = useState<ReadonlySet<string>>(() => new Set(readJsonPref("collapsedProjects", [], isStrings)));
  const [showAll, setShowAll] = useState<ReadonlySet<string>>(new Set());
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => writePref("collapsedProjects", JSON.stringify([...collapsed])), [collapsed]);
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(t);
  }, []);
  // The open chat is always findable: its project expands and its row scrolls into view.
  const activeProject = p.chats.find((c) => c.id === p.activeChat)?.projectId ?? null;
  useEffect(() => {
    if (!activeProject) return;
    setCollapsed((s) => {
      if (!s.has(activeProject)) return s;
      const n = new Set(s);
      n.delete(activeProject);
      return n;
    });
    const id = window.requestAnimationFrame(() =>
      document.querySelector(".side-chat.on")?.scrollIntoView({ block: "nearest" }),
    );
    return () => window.cancelAnimationFrame(id);
  }, [activeProject, p.activeChat]);
  const toggle = (set: ReadonlySet<string>, id: string) => {
    const n = new Set(set);
    if (n.has(id)) n.delete(id);
    else n.add(id);
    return n;
  };

  const chatsOf = new Map<string, Chat[]>();
  for (const c of p.chats) {
    const list = chatsOf.get(c.projectId);
    if (list) list.push(c);
    else chatsOf.set(c.projectId, [c]);
  }

  return (
    <aside className="sidebar" aria-label="Sidebar">
      <div className="brand">
        <BrandMark size={22} />
        <span className="brand-text">
          <span className="brand-name">Nodal</span>
          <span className="brand-tag">Agent OS for Claude</span>
        </span>
      </div>
      <button type="button" className="btn side-search" onClick={p.onOpenPalette} aria-keyshortcuts="Meta+K">
        <span className="ellipsis">Search or run…</span>
        <Keys keys={["⌘", "K"]} />
      </button>
      <button type="button" className="btn side-new-project" onClick={p.onNewProject}>
        <SideIcon name="plus" size={13} />
        <span className="side-item-label">New project</span>
      </button>

      <div className="side-mode" role="group" aria-label="View">
        {(["chat", "tasks"] as const).map((m) => (
          <button
            key={m}
            type="button"
            className={`side-mode-btn ${p.mode === m ? "on" : ""}`}
            aria-pressed={p.mode === m}
            aria-keyshortcuts={m === "chat" ? "Meta+1" : "Meta+2"}
            onClick={() => p.onMode(m)}
          >
            <SideIcon name={m === "chat" ? "chat" : "board"} />
            {m === "chat" ? "Chats" : "Tasks"}
          </button>
        ))}
      </div>

      {p.mode === "chat" ? (
        <nav className="side-groups" aria-label="Chats by project">
          {p.projects.map((proj) => {
            const all = chatsOf.get(proj.id) ?? [];
            const open = !collapsed.has(proj.id);
            const recent = all.slice(0, RECENT);
            const older = all.find((c, i) => i >= RECENT && c.id === p.activeChat);
            const list = showAll.has(proj.id) ? all : older ? [...recent, older] : recent;
            const repoId = p.defaultRepo(proj.id);
            return (
              <section key={proj.id} className="side-group" aria-label={proj.name}>
                <div className="side-group-head">
                  <button
                    type="button"
                    className="side-group-toggle"
                    aria-expanded={open}
                    onClick={() => setCollapsed((s) => toggle(s, proj.id))}
                  >
                    <span className="project-dot" style={{ ["--project-color" as string]: proj.color }} aria-hidden />
                    <span className="side-group-name ellipsis">{proj.name}</span>
                    <span className="side-count num">{all.length || ""}</span>
                    <span className={`side-group-chev ${open ? "open" : ""}`}>
                      <SideIcon name="chevron" size={12} />
                    </span>
                  </button>
                  <button
                    type="button"
                    className="icon-btn side-group-add"
                    aria-label={`New chat in ${proj.name}`}
                    title={`New chat in ${proj.name}`}
                    disabled={!repoId}
                    onClick={() => p.onNewChat(proj.id)}
                  >
                    <SideIcon name="plus" size={13} />
                  </button>
                </div>
                {open && (
                  <>
                    {list.map((c) => (
                      <ChatRow
                        key={c.id}
                        chat={c}
                        repoId={c.repoId ?? repoId}
                        on={p.activeChat === c.id}
                        live={p.liveChats.has(c.id)}
                        unread={p.unreadChats.has(c.id)}
                        now={now}
                        onOpen={() => p.onOpenChat(c)}
                        onDelete={() => p.onDeleteChat(c)}
                      />
                    ))}
                    {all.length === 0 && (
                      <button type="button" className="side-group-empty" disabled={!repoId} onClick={() => p.onNewChat(proj.id)}>
                        {repoId ? "Start the first chat" : "Add a repo to chat"}
                      </button>
                    )}
                    {all.length > RECENT && (
                      <button type="button" className="side-group-more" onClick={() => setShowAll((s) => toggle(s, proj.id))}>
                        {showAll.has(proj.id) ? "Show fewer" : `${all.length - RECENT} older chats`}
                      </button>
                    )}
                  </>
                )}
              </section>
            );
          })}
          {p.projects.length === 0 && <div className="side-empty">No projects yet.</div>}
        </nav>
      ) : (
        <>
          <button type="button" className="btn side-primary" disabled={!p.canNewTask} onClick={p.onNewTask}>
            <SideIcon name="plus" size={13} />
            <span className="side-item-label">New task</span>
          </button>
          <nav className="side-projects" aria-label="Boards">
            <button
              type="button"
              className={`side-item ${p.activeBoard === null ? "on" : ""}`}
              aria-current={p.activeBoard === null ? "page" : undefined}
              onClick={() => p.onOpenBoard(null)}
            >
              <span className="tab-dot-all" aria-hidden />
              <span className="side-item-label ellipsis">All projects</span>
              {p.openTotal > 0 && <span className="side-count num">{p.openTotal} open</span>}
            </button>
            <div className="side-heading">
              <h2 className="side-heading-label">Projects</h2>
            </div>
            {p.projects.map((proj) => {
              const on = p.activeBoard === proj.id;
              const open = p.openByProject.get(proj.id) ?? 0;
              return (
                <div key={proj.id} className={`side-project-row ${on ? "on-soft" : ""}`}>
                  <button
                    type="button"
                    className="side-project-pick"
                    aria-current={on ? "page" : undefined}
                    onClick={() => p.onOpenBoard(proj.id)}
                  >
                    <span className="project-dot" style={{ ["--project-color" as string]: proj.color }} aria-hidden />
                    <span className="side-item-label ellipsis">{proj.name}</span>
                    {p.needYouByProject.has(proj.id) && <span className="side-need">Needs you</span>}
                    {open > 0 && <span className="side-count num">{open}</span>}
                  </button>
                  <button
                    type="button"
                    className="icon-btn side-project-btn"
                    title={p.mappingDrift.has(proj.id) ? "Source states changed: review the mapping" : `${proj.name} settings`}
                    aria-label={`${proj.name} settings`}
                    onClick={() => p.onProjectSettings(proj.id)}
                  >
                    {p.mappingDrift.has(proj.id) ? <span className="dot dot-sm tone-warn" aria-hidden /> : <SideIcon name="settings" size={13} />}
                  </button>
                </div>
              );
            })}
            {p.projects.length === 0 && <div className="side-empty">No projects yet.</div>}
          </nav>
        </>
      )}

      <div className="side-foot">
        <button
          type="button"
          className={`side-item ${p.settingsOpen ? "on" : ""}`}
          aria-current={p.settingsOpen ? "page" : undefined}
          aria-keyshortcuts="Meta+,"
          onClick={p.onSettings}
        >
          <SideIcon name="settings" />
          <span className="side-item-label">Settings</span>
          <Tooltip>
            <TooltipTrigger asChild>
              <span className="side-foot-status" role="status" aria-label={p.provider.label}>
                <span className={`dot dot-sm tone-${p.provider.tone}`} aria-hidden />
              </span>
            </TooltipTrigger>
            <TooltipContent side="top">{p.provider.label}</TooltipContent>
          </Tooltip>
        </button>
        {p.updateAvailable ? (
          <button
            type="button"
            className="side-version side-update"
            disabled={p.updating || p.checkingUpdates}
            onClick={p.onUpdate}
            aria-label={p.version ? `Update to v${p.updateAvailable} (installed v${p.version})` : undefined}
          >
            {p.updating ? "Updating…" : `Update to v${p.updateAvailable}`}
          </button>
        ) : (
          p.version && <span className="side-version">v{p.version}</span>
        )}
      </div>
    </aside>
  );
}

interface ChatRowProps {
  chat: Chat;
  repoId: string | null;
  on: boolean;
  live: boolean;
  unread: boolean;
  now: number;
  onOpen: () => void;
  onDelete: () => void;
}

function ChatRow({ chat, repoId, on, live, unread, now, onOpen, onDelete }: ChatRowProps) {
  // Where the chat works now (it may have moved into a worktree); the repo's branch until known.
  const ws = useChatWorkspace(chat.id).data;
  const repoBranch = useRepoBranch(ws ? null : repoId).data ?? null;
  const branch = ws?.branch ?? repoBranch;
  const changes = ws && ws.files > 0 ? `+${ws.additions} −${ws.deletions}` : null;
  const title = chatTitle(chat);
  return (
    <div className={`side-chat ${on ? "on" : ""} ${unread ? "unread" : ""}`}>
      <button
        type="button"
        className="side-chat-pick"
        aria-current={on ? "page" : undefined}
        title={chat.sessionId ? `claude --resume ${chat.sessionId}` : title}
        onClick={onOpen}
      >
        <span className={`side-chat-dot ${live ? "live pulse" : unread ? "unread" : ""}`} aria-hidden />
        <span className="side-chat-text">
          <span className="side-chat-title ellipsis">{title}</span>
          {branch && (
            <span className="side-chat-branch" title={ws?.cwd ?? undefined}>
              <SideIcon name="branch" size={11} />
              <span className="ellipsis">{branch}</span>
              {ws?.isWorktree && <span className="side-chat-wt">worktree</span>}
              {changes && <span className="side-chat-diff num">{changes}</span>}
            </span>
          )}
        </span>
        {live ? (
          <span className="side-chat-meta live">Working</span>
        ) : (
          <span className="side-chat-meta num">
            {unread && <span className="sr-only">Unread, </span>}
            {shortAgo(chat.updatedAt, now)}
          </span>
        )}
      </button>
      <button type="button" className="icon-btn side-chat-del" aria-label={`Delete ${title}`} title="Delete chat" onClick={onDelete}>
        <SideIcon name="trash" size={12} />
      </button>
    </div>
  );
}
