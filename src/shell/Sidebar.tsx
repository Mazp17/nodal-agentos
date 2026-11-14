import { useEffect, useState, type ReactNode } from "react";
import { ContextMenu } from "radix-ui";
import { useChatWorkspace, useRepoBranch } from "../domain/hooks/branches";
import type { Chat, Project } from "../domain/types";
import { useTask } from "../domain/hooks/store";
import { chatTitle, shortAgo } from "../features/chats/model";
import { taskTag } from "../features/tasks/status";
import { BrandMark } from "../ui/BrandMark";
import { Kbd, KbdGroup } from "../ui/Kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/Tooltip";
import { readJsonPref, writePref } from "./storage";
import { Resizer, clampWidth } from "./Resizer";
import type { ProjectSection } from "./useNav";
import { projectColor } from "../features/projects/create";

export type SideMode = "chat" | "tasks";
type IconName = "update" | "overview" | "chat" | "board" | "settings" | "plus" | "chevron" | "branch" | "trash" | "pin";

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
      {name === "update" && (
        <>
          <circle cx="7" cy="7" r="5.5" />
          <path d="M7 4v5M4.8 6.8L7 9l2.2-2.2" />
        </>
      )}
      {name === "overview" && (
        <>
          <rect x="1.5" y="1.5" width="4.5" height="4.5" rx="1" />
          <rect x="8" y="1.5" width="4.5" height="4.5" rx="1" />
          <rect x="1.5" y="8" width="4.5" height="4.5" rx="1" />
          <rect x="8" y="8" width="4.5" height="4.5" rx="1" />
        </>
      )}
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
      {name === "pin" && <path d="M5 1.5h4M5.5 1.5v4L3.5 8h7l-2-2.5v-4M7 8v4.5" />}
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
  overviewOpen: boolean;
  onOverview: () => void;
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
  onPinChat: (chat: Chat) => void;
  onDeleteChat: (chat: Chat) => void;
  onOpenBoard: (projectId: string | null) => void;
  onNewTask: () => void;
  onProjectSettings: (projectId: string, section?: ProjectSection) => void;
  onSettings: () => void;
  onNewProject: () => void;
  onOpenPalette: () => void;
}

const SIDE_MIN = 232;
const SIDE_MAX = 560;
const SIDE_DEFAULT = 248;

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

  // Resizable and collapsible (dragged to the edge, ⌘B, or the active rail icon), remembered.
  const [width, setWidth] = useState(() =>
    clampWidth(readJsonPref("sideWidth", SIDE_DEFAULT, (v): v is number => typeof v === "number"), SIDE_MIN, SIDE_MAX),
  );
  const [hidden, setHidden] = useState(() => readJsonPref("sideCollapsed", false, (v): v is boolean => typeof v === "boolean"));
  useEffect(() => writePref("sideWidth", JSON.stringify(width)), [width]);
  useEffect(() => writePref("sideCollapsed", JSON.stringify(hidden)), [hidden]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !(e.metaKey || e.ctrlKey) || e.shiftKey || e.altKey || e.key.toLowerCase() !== "b") return;
      e.preventDefault();
      setHidden((h) => !h);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const showChats = p.mode === "chat";
  const railOn = (m: "overview" | SideMode) =>
    m === "overview" ? p.overviewOpen : !p.overviewOpen && !p.settingsOpen && p.mode === m;
  const needYou = p.needYouByProject.size;
  const rail: { id: "overview" | SideMode; label: string; key: string; icon: IconName; badge: number; tone: "live" | "need" }[] = [
    { id: "overview", label: "Overview", key: "3", icon: "overview", badge: p.liveChats.size, tone: "live" },
    { id: "chat", label: "Chats", key: "1", icon: "chat", badge: p.unreadChats.size, tone: "live" },
    { id: "tasks", label: "Tasks", key: "2", icon: "board", badge: needYou, tone: "need" },
  ];

  return (
    <>
      {/* The places, as an activity rail: the panel beside it shows only the one picked. */}
      <nav className="rail" aria-label="Places">
        <span className="rail-brand" title="Nodal · Agent OS for Claude">
          <BrandMark size={24} />
        </span>
        {rail.map((r) => {
          const on = railOn(r.id);
          return (
            <button
              key={r.id}
              type="button"
              className={`rail-btn ${on ? "on" : ""}`}
              aria-current={on ? "page" : undefined}
              aria-label={r.badge ? `${r.label}, ${r.badge} ${r.tone === "need" ? "need you" : r.id === "chat" ? "unread" : "working"}` : r.label}
              aria-keyshortcuts={`Meta+${r.key}`}
              title={`${r.label} (⌘${r.key})`}
              onClick={() => {
                if (r.id === "overview") return p.onOverview();
                // Like an editor's activity bar: the place already showing hides or shows its list.
                if (on) return setHidden((h) => !h);
                setHidden(false);
                p.onMode(r.id);
              }}
            >
              <SideIcon name={r.icon} size={18} />
              {r.badge > 0 && <span className={`rail-badge rail-badge-${r.tone}`}>{r.badge > 9 ? "9+" : r.badge}</span>}
            </button>
          );
        })}
        <span className="rail-spacer" />
        {p.updateAvailable && (
          <button
            type="button"
            className="rail-btn rail-update"
            disabled={p.updating || p.checkingUpdates}
            onClick={p.onUpdate}
            title={p.updating ? "Updating…" : `Update to v${p.updateAvailable}${p.version ? ` (installed v${p.version})` : ""}`}
            aria-label={`Update to v${p.updateAvailable}`}
          >
            <SideIcon name="update" size={18} />
          </button>
        )}
        <button
          type="button"
          className={`rail-btn ${p.settingsOpen ? "on" : ""}`}
          aria-current={p.settingsOpen ? "page" : undefined}
          aria-keyshortcuts="Meta+,"
          aria-label="Settings"
          title={p.version ? `Settings (⌘,) · v${p.version}` : "Settings (⌘,)"}
          onClick={p.onSettings}
        >
          <SideIcon name="settings" size={18} />
        </button>
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="rail-status" role="status" aria-label={p.provider.label}>
              <span className={`dot dot-sm tone-${p.provider.tone}`} aria-hidden />
            </span>
          </TooltipTrigger>
          <TooltipContent side="right">{p.provider.label}</TooltipContent>
        </Tooltip>
      </nav>

      {/* Overview and Settings are pages of their own: no list beside them. */}
      {!p.overviewOpen && !p.settingsOpen && !hidden && (
    <aside className="sidebar" aria-label={showChats ? "Chats" : "Tasks"} style={{ width }}>
      <Resizer
        edge="right"
        label="Resize sidebar"
        width={width}
        min={SIDE_MIN}
        max={SIDE_MAX}
        initial={SIDE_DEFAULT}
        onWidth={setWidth}
        onCollapse={setHidden}
      />
      <div className="side-head">
        <h2 className="side-title">{showChats ? "Chats" : "Tasks"}</h2>
        <button
          type="button"
          className="icon-btn side-head-add"
          aria-label="New project"
          title="New project"
          onClick={p.onNewProject}
        >
          <SideIcon name="plus" size={13} />
        </button>
      </div>
      <button type="button" className="btn side-search" onClick={p.onOpenPalette} aria-keyshortcuts="Meta+K">
        <span className="ellipsis">Search chats, files, tasks…</span>
        <Keys keys={["⌘", "K"]} />
      </button>

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
                <ProjectMenu p={p} projectId={proj.id} canChat={!!repoId}>
                <div className="side-group-head">
                  <button
                    type="button"
                    className="side-group-toggle"
                    aria-expanded={open}
                    onClick={() => setCollapsed((s) => toggle(s, proj.id))}
                  >
                    <span className="project-dot" style={{ ["--project-color" as string]: projectColor(proj.color) }} aria-hidden />
                    <span className="side-group-name ellipsis">{proj.name}</span>
                    {(() => {
                      // Seen even with the project folded: how many of its chats are answering now.
                      const live = all.filter((c) => p.liveChats.has(c.id)).length;
                      return live > 0 ? (
                        <span className="side-group-live">
                          <span className="side-live-dot" aria-hidden />
                          {live} running
                        </span>
                      ) : null;
                    })()}
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
                </ProjectMenu>
                {open && (
                  <>
                    {list.map((c) => (
                      <ChatRow
                        key={c.id}
                        chat={c}
                        repoId={c.repoId ?? repoId}
                        projectKey={proj.key}
                        on={p.activeChat === c.id}
                        live={p.liveChats.has(c.id)}
                        unread={p.unreadChats.has(c.id)}
                        now={now}
                        onOpen={() => p.onOpenChat(c)}
                        onPin={() => p.onPinChat(c)}
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
                <ProjectMenu key={proj.id} p={p} projectId={proj.id} canChat={!!p.defaultRepo(proj.id)}>
                <div className={`side-project-row ${on ? "on-soft" : ""}`}>
                  <button
                    type="button"
                    className="side-project-pick"
                    aria-current={on ? "page" : undefined}
                    onClick={() => p.onOpenBoard(proj.id)}
                  >
                    <span className="project-dot" style={{ ["--project-color" as string]: projectColor(proj.color) }} aria-hidden />
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
                </ProjectMenu>
              );
            })}
            {p.projects.length === 0 && <div className="side-empty">No projects yet.</div>}
          </nav>
        </>
      )}

    </aside>
      )}
    </>
  );
}

interface ChatRowProps {
  chat: Chat;
  repoId: string | null;
  /** Project key, for the linked task's local id. */
  projectKey: string;
  on: boolean;
  live: boolean;
  unread: boolean;
  now: number;
  onOpen: () => void;
  onPin: () => void;
  onDelete: () => void;
}

function ChatRow({ chat, repoId, projectKey, on, live, unread, now, onOpen, onPin, onDelete }: ChatRowProps) {
  // Where the chat works now (it may have moved into a worktree); the repo's branch until known.
  const ws = useChatWorkspace(chat.id).data;
  const repoBranch = useRepoBranch(ws ? null : repoId).data ?? null;
  const branch = ws?.branch ?? repoBranch;
  const changes =
    ws && ws.files > 0 ? (
      <>
        <span className="side-chat-diff-add">+{ws.additions}</span> <span className="side-chat-diff-del">−{ws.deletions}</span>
      </>
    ) : null;
  const title = chatTitle(chat);
  const task = useTask(chat.taskId).data;
  const pinned = chat.pinnedAt != null;
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
          <span className="side-chat-title ellipsis">
            {pinned && (
              <span className="side-chat-pinned" title="Pinned">
                <SideIcon name="pin" size={11} />
                <span className="sr-only">Pinned, </span>
              </span>
            )}
            {task && <span className="side-chat-task mono">{taskTag(task, projectKey)}</span>}
            {title}
          </span>
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
      <button
        type="button"
        className="icon-btn side-chat-pin"
        aria-label={`${pinned ? "Unpin" : "Pin"} ${title}`}
        aria-pressed={pinned}
        title={pinned ? "Unpin chat" : "Pin chat"}
        onClick={onPin}
      >
        <SideIcon name="pin" size={12} />
      </button>
      <button type="button" className="icon-btn side-chat-del" aria-label={`Delete ${title}`} title="Delete chat" onClick={onDelete}>
        <SideIcon name="trash" size={12} />
      </button>
    </div>
  );
}

/** Right-click on a project (either sidebar mode): its chat, board and settings pages. */
function ProjectMenu({ p, projectId, canChat, children }: { p: Props; projectId: string; canChat: boolean; children: ReactNode }) {
  const settings = (section: ProjectSection) => () => p.onProjectSettings(projectId, section);
  return (
    <ContextMenu.Root>
      <ContextMenu.Trigger asChild>{children}</ContextMenu.Trigger>
      <ContextMenu.Portal>
        <ContextMenu.Content className="menu tab-menu" collisionPadding={8}>
          <ContextMenu.Item className="menu-item" disabled={!canChat} onSelect={() => p.onNewChat(projectId)}>
            New chat
          </ContextMenu.Item>
          <ContextMenu.Item className="menu-item" onSelect={() => p.onOpenBoard(projectId)}>
            Open board
          </ContextMenu.Item>
          <ContextMenu.Separator className="menu-sep" />
          <ContextMenu.Item className="menu-item" onSelect={settings("general")}>
            Project settings
          </ContextMenu.Item>
          <ContextMenu.Item className="menu-item" onSelect={settings("repos")}>
            Repositories
          </ContextMenu.Item>
          <ContextMenu.Item className="menu-item" onSelect={settings("worktrees")}>
            Worktrees
          </ContextMenu.Item>
          <ContextMenu.Item className="menu-item" onSelect={settings("sources")}>
            Sources
          </ContextMenu.Item>
          <ContextMenu.Item className="menu-item" onSelect={settings("agents")}>
            Agents
          </ContextMenu.Item>
        </ContextMenu.Content>
      </ContextMenu.Portal>
    </ContextMenu.Root>
  );
}
