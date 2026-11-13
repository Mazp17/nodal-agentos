import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { deleteChat, listRepoFiles, onCloseTab } from "../domain/api";
import { useChatWorkspace, useRepoWorktrees } from "../domain/hooks/branches";
import { useAllChats } from "../domain/hooks/chats";
import { useProjects } from "../domain/hooks/projects";
import { useProviderStatus, useSourceLinks } from "../domain/hooks/providers";
import { invalidate } from "../domain/hooks/store";
import { taskKey, type Chat, type Project } from "../domain/types";
import { BoardView } from "../features/board";
import { ChatView } from "../features/chats";
import { chatTitle } from "../features/chats/model";
import { setInAppNotice } from "../features/chats/alerts";
import { forgetChat, pruneUnread, sendToChat, useRunStates, useUnreadChats } from "../features/chats/stream";
import { FilesPanel, type FilesMode } from "../features/chats/FilesPanel";
import { setDraft, useDirtyTabs } from "../features/chats/drafts";
import { CommitView, FileDiffView, FileView } from "../features/chats/FileView";
import { OverviewView } from "../features/overview/OverviewView";
import { Onboarding } from "../features/onboarding/Onboarding";
import { CreateProjectDialog } from "../features/projects/CreateProjectDialog";
import { ProjectSettings } from "../features/projects/ProjectSettings";
import { ImportDialog } from "../features/providers";
import { useLegacyImport } from "../features/settings/legacyImport";
import { SettingsView } from "../features/settings/SettingsView";
import { NewTaskDialog, OpenChatContext, TaskPanel } from "../features/tasks";
import { useUpdates } from "../features/updates/useUpdates";
import { CommandPalette, type PaletteItem } from "./palette/CommandPalette";
import { Sidebar, type ProviderFoot, type SideMode } from "./Sidebar";
import { TabBar, type TabItem } from "./TabBar";
import { commitTab, draftProject, draftTab, fileTab, parseFileTab, useTabs } from "./useTabs";
import { useConfirm } from "../ui/ConfirmDialog";
import { EmptyState } from "../ui/EmptyState";
import { useToast } from "../ui/Toasts";
import { Topbar } from "./Topbar";
import { PAGE_TITLE, useNav, type Page } from "./useNav";
import { readJsonPref, writePref } from "./storage";
import { useWorkStatus } from "./useWorkStatus";
import "./shell.css";
import { projectColor } from "../features/projects/create";

/** ⌘1 Chats, ⌘2 Tasks, ⌘3 Overview. */
const SHORTCUT_PAGES: Record<string, Page> = { "1": "chat", "2": "board", "3": "overview" };
/** Board tab id of the pinned "All projects". */
const ALL_BOARD = "__all";

export function AppShell() {
  const ctx = useProjects();
  const provider = useProviderStatus("linear");
  const nav = useNav();
  const toast = useToast();
  const work = useWorkStatus();
  const legacy = useLegacyImport();
  const updates = useUpdates();

  const [forceOnboarding, setForceOnboarding] = useState(false);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [createProject, setCreateProject] = useState(false);
  const [newTask, setNewTask] = useState<{ projectId: string | null } | null>(null);
  const [importFor, setImportFor] = useState<string | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const allChats = useAllChats(true);
  const tabs = useTabs();
  const ask = useConfirm();
  const runStates = useRunStates();
  // The files panel comes back as it was left: open or not, which view and which checkout.
  const [filesOpen, setFilesOpen] = useState(() => readJsonPref("filesPanel", false, (v): v is boolean => typeof v === "boolean"));
  const [filesMode, setFilesMode] = useState<FilesMode>(() =>
    readJsonPref<FilesMode>("filesMode", "tree", (v): v is FilesMode => v === "tree" || v === "changes" || v === "agents"),
  );
  /** Checkouts picked by hand in the files panel, per tab (`null`: the repo's own). */
  const [wtPicked, setWtPicked] = useState<Readonly<Record<string, string | null>>>({});
  useEffect(() => writePref("filesPanel", JSON.stringify(filesOpen)), [filesOpen]);
  useEffect(() => writePref("filesMode", JSON.stringify(filesMode)), [filesMode]);
  // The chat's working folder: the files panel follows it into a worktree.
  const activeChatId = tabs.chat && !parseFileTab(tabs.chat) && !draftProject(tabs.chat) ? tabs.chat : null;
  const chatCwd = useChatWorkspace(activeChatId).data?.cwd ?? null;
  const dirtyTabs = useDirtyTabs();
  const chatById = useMemo(() => new Map((allChats.data ?? []).map((c) => [c.id, c])), [allChats.data]);
  /** Project of a tab in the chat bar (a draft carries it, a chat has it, a file's repo has it). */
  const chatProjectOf = (tab: string | null): string | null => {
    if (!tab) return null;
    const file = parseFileTab(tab);
    if (file) return ctx.repoById.get(file.repoId)?.projectId ?? null;
    return draftProject(tab) ?? chatById.get(tab)?.projectId ?? null;
  };

  // Which checkout the files panel shows, per tab: the one picked by hand there, else where the
  // tab works (the chat's folder, the file's worktree); a new chat is the repo's own checkout.
  const tabFile = tabs.chat ? parseFileTab(tabs.chat) : null;
  const tabRepos = ctx.reposOf(chatProjectOf(tabs.chat) ?? "");
  const panelRepoId =
    (tabRepos.find((r) => r.id === (tabFile ? tabFile.repoId : tabs.chat ? chatById.get(tabs.chat)?.repoId : null)) ?? tabRepos[0])?.id ?? null;
  const panelWorktrees = useRepoWorktrees(panelRepoId).data;
  const followed = tabFile ? tabFile.worktree : chatCwd;
  const autoWorktree = (() => {
    if (!followed || !panelWorktrees) return null;
    const hit = panelWorktrees
      .filter((w) => followed === w.path || followed.startsWith(`${w.path}/`))
      .sort((a, b) => b.path.length - a.path.length)[0];
    return hit && !hit.isMain ? hit.path : null;
  })();
  const tabKey = tabs.chat ?? "";
  const filesWorktree = tabKey in wtPicked ? wtPicked[tabKey]! : autoWorktree;
  const setFilesWorktree = useCallback((w: string | null) => setWtPicked((m) => ({ ...m, [tabKey]: w })), [tabKey]);

  const { route } = nav;
  const project = route.projectId ? (ctx.projectById.get(route.projectId) ?? null) : null;
  const visible = route;
  const projectRepos = project ? ctx.reposOf(project.id) : [];

  // Chat answers notify from any page, not only while the Chat page is open: `ensureChatAttached`
  // (called once a chat is sent to, from the Chat page) keeps its channel attached regardless of
  // which page is showing afterward.
  const unreadChats = useUnreadChats();
  useEffect(() => {
    if (ctx.loaded && !ctx.error) pruneUnread(new Set(ctx.projectById.keys()));
  }, [ctx.loaded, ctx.error, ctx.projectById]);
  useEffect(() => {
    setInAppNotice((title, body) => toast(title, body, "info"));
    return () => setInAppNotice(null);
  }, [toast]);

  // Project deleted (from here or elsewhere): go back to "All projects".
  const { forgetProject } = nav;
  useEffect(() => {
    if (ctx.loaded && !ctx.error && route.projectId && !ctx.projectById.has(route.projectId)) {
      forgetProject(route.projectId);
    }
  }, [ctx.loaded, ctx.error, ctx.projectById, route.projectId, forgetProject]);

  // Dock badge: what needs the user (no number when 0).
  const needYou = work.blocked.length;
  useEffect(() => {
    getCurrentWindow()
      .setBadgeCount(needYou || undefined)
      .catch((e: unknown) => console.error("setBadgeCount", e));
  }, [needYou]);

  // Last visited project: target of ⌘2 from a global view.
  const lastProjectId = useRef<string | null>(null);
  useEffect(() => {
    if (route.projectId) lastProjectId.current = route.projectId;
  }, [route.projectId]);


  // Current project's sources with Linear connected: they enable "Import".
  const projectId = project?.id ?? null;
  const links = useSourceLinks(projectId);
  const canImport = projectId !== null && provider.connection === "connected" && (links.data?.length ?? 0) > 0;
  // No network: `pendingStateChanges` is left by the last sync on each source.
  const allLinks = useSourceLinks(null);
  const mappingDrift = useMemo(
    () => new Set((allLinks.data ?? []).filter((l) => l.pendingStateChanges).map((l) => l.projectId)),
    [allLinks.data],
  );

  const overlayOpen = paletteOpen || createProject || newTask !== null || importFor !== null;

  const openTask = useCallback((id: string) => {
    setPaletteOpen(false);
    setTaskId(id);
  }, []);
  const { go: navGo } = nav;
  const go = useCallback(
    (page: Page, pid: string | null = null) => {
      setTaskId(null);
      setPaletteOpen(false);
      navGo(page, pid);
    },
    [navGo],
  );
  const openNewTask = useCallback(() => {
    setPaletteOpen(false);
    setNewTask({ projectId: project?.id ?? null });
  }, [project]);

  const { setProjectSection, setSettingsSection } = nav;
  const openRepoSettings = useCallback(
    (pid: string) => {
      setProjectSection("repos");
      go("project-settings", pid);
    },
    [setProjectSection, go],
  );
  const openIntegrations = useCallback(() => {
    setSettingsSection("integrations");
    go("settings");
  }, [setSettingsSection, go]);

  /** Project for ⌘2 from a global view: the current one, the last visited, or the first. */
  const fallbackProject = (): Project | null =>
    project ??
    (lastProjectId.current ? ctx.projectById.get(lastProjectId.current) : undefined) ??
    ctx.projects[0] ??
    null;

  // Last board shown: target of the Tasks switch.
  const lastBoard = useRef<string | null>(null);
  useEffect(() => {
    if (route.page === "board") lastBoard.current = route.projectId;
  }, [route.page, route.projectId]);

  // Sidebar mode follows the page, but global pages (settings) keep whichever mode was showing.
  const sideMode = useRef<SideMode>("chat");
  if (visible.page === "chat") sideMode.current = "chat";
  else if (visible.page === "board") sideMode.current = "tasks";

  const goMode = (m: SideMode) => {
    if (m === "tasks") {
      go("board", lastBoard.current);
      return;
    }
    const pid = chatProjectOf(tabs.chat) ?? fallbackProject()?.id ?? null;
    if (pid) go("chat", pid);
  };
  const openChatTab = (c: Chat) => {
    tabs.openChat(c.id);
    go("chat", c.projectId);
  };
  const newChat = (pid: string) => {
    tabs.openChat(draftTab(pid));
    go("chat", pid);
  };
  const pickChatTab = (tab: string) => {
    tabs.openChat(tab);
    const pid = chatProjectOf(tab);
    if (pid) go("chat", pid);
  };
  /** Closing file tabs with unsaved edits asks first; on yes, their edits are dropped. */
  const discardOk = async (ids: string[]) => {
    const dirty = ids.filter((t) => dirtyTabs.has(t));
    if (dirty.length === 0) return true;
    const one = dirty.length === 1 ? parseFileTab(dirty[0]!)?.path.split("/").pop() : null;
    const ok = await ask({
      title: one ? `Discard your changes to ${one}?` : `Discard your changes to ${dirty.length} files?`,
      body: "They haven't been saved.",
      confirmLabel: "Discard",
    });
    if (ok) for (const t of dirty) setDraft(t, null);
    return ok;
  };
  const closeChatTabs = async (ids: string[]) => {
    if (!(await discardOk(ids))) return;
    const pid = chatProjectOf(tabs.closeChats(ids));
    if (pid && route.page === "chat") go("chat", pid);
  };
  const closeChatTab = (tab: string) => {
    if (dirtyTabs.has(tab)) {
      void closeChatTabs([tab]);
      return;
    }
    const next = tabs.closeChat(tab);
    const pid = chatProjectOf(next);
    if (pid && route.page === "chat") go("chat", pid);
  };
  const removeChat = async (c: Chat) => {
    const ok = await ask({
      title: `Delete “${chatTitle(c)}”?`,
      body: "It leaves the chats list. The Claude Code session stays on disk and can still be resumed with claude --resume.",
      confirmLabel: "Delete chat",
    });
    if (!ok) return;
    try {
      await deleteChat(c.id);
      forgetChat(c.id);
      if (tabs.chats.includes(c.id)) closeChatTab(c.id);
      void invalidate("chats");
    } catch (e) {
      toast("Couldn't delete the chat", String(e), "danger");
    }
  };
  const openBoard = (pid: string | null) => {
    if (pid) tabs.openBoard(pid);
    go("board", pid);
  };
  const closeBoardTab = (pid: string) => {
    tabs.closeBoard(pid);
    if (route.page === "board" && route.projectId === pid) go("board", null);
  };

  // A board opened from elsewhere (palette, a new project) gets its tab.
  const { openBoard: addBoardTab, prune: pruneTabs } = tabs;
  useEffect(() => {
    if (route.page === "board" && route.projectId) addBoardTab(route.projectId);
  }, [route.page, route.projectId, addBoardTab]);
  // Tabs of deleted chats and projects go away.
  useEffect(() => {
    if (!allChats.data || !ctx.loaded || ctx.error) return;
    const ids = new Set(allChats.data.map((c) => c.id));
    pruneTabs(
      (t) => {
        const file = parseFileTab(t);
        if (file) return ctx.repoById.has(file.repoId);
        const dp = draftProject(t);
        return dp ? ctx.projectById.has(dp) : ids.has(t);
      },
      (id) => ctx.projectById.has(id),
    );
  }, [allChats.data, ctx.loaded, ctx.error, ctx.projectById, pruneTabs]);

  const liveChats = useMemo(() => new Set([...runStates].filter(([, st]) => st === "busy").map(([id]) => id)), [runStates]);
  const openByProject = useMemo(() => {
    const m = new Map<string, number>();
    for (const t of work.tasks) if (t.status !== "done" && t.status !== "canceled") m.set(t.projectId, (m.get(t.projectId) ?? 0) + 1);
    return m;
  }, [work.tasks]);
  const needYouByProject = useMemo(() => new Set(work.blocked.map((t) => t.projectId)), [work.blocked]);

  const showOnboarding = ctx.loaded && (ctx.projects.length === 0 || forceOnboarding) && !(ctx.error && ctx.projects.length === 0);

  // The handler changes every render; the listener is registered once and calls the latest.
  const onKeyRef = useRef<(e: KeyboardEvent) => void>(() => {});
  useLayoutEffect(() => {
    onKeyRef.current = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing) return;
      // Outside the shell (loading, error, onboarding) there are no shortcuts.
      if (!ctx.loaded || showOnboarding || (ctx.error && ctx.projects.length === 0)) return;
      const mod = e.metaKey || e.ctrlKey;
      if (mod && e.key.toLowerCase() === "k") {
        e.preventDefault();
        if (createProject || newTask || importFor) return;
        setPaletteOpen((o) => !o);
        return;
      }
      if (mod && !e.shiftKey && !e.altKey && (e.key in SHORTCUT_PAGES || e.key === ",")) {
        e.preventDefault();
        if (overlayOpen) return;
        if (e.key === ",") {
          go("settings");
          return;
        }
        const page = SHORTCUT_PAGES[e.key]!;
        if (page === "chat") goMode("chat");
        else if (page === "board") goMode("tasks");
        else go(page, visible.projectId ?? null);
        return;
      }
      // Cascading Esc: the palette and dialogs catch it first (useFocusTrap); what's left
      // here is the task drawer.
      if (e.key === "Escape" && taskId) {
        e.preventDefault();
        setTaskId(null);
      }
    };
  });
  // ⌘W: the task drawer if open, else the tab in view (chat/file tab, or the board's).
  const closeCurrentRef = useRef(() => {});
  useLayoutEffect(() => {
    closeCurrentRef.current = () => {
      if (overlayOpen) return;
      if (taskId) setTaskId(null);
      else if (visible.page === "chat" && tabs.chat) closeChatTab(tabs.chat);
      else if (visible.page === "board" && visible.projectId) closeBoardTab(visible.projectId);
    };
  });
  useEffect(() => {
    const off = onCloseTab(() => closeCurrentRef.current());
    return () => void off.then((f) => f());
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => onKeyRef.current(e);
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // ---- Palette ----
  // Quick open: the files of the project in view (its repos' checkouts), read when the palette opens.
  const [paletteFiles, setPaletteFiles] = useState<{ repoId: string; projectId: string; path: string }[]>([]);
  const filesProject = chatProjectOf(tabs.chat) ?? visible.projectId ?? ctx.projects[0]?.id ?? null;
  const filesRepoIds = filesProject ? ctx.reposOf(filesProject).map((r) => r.id).join(",") : "";
  useEffect(() => {
    if (!paletteOpen || !filesProject || !filesRepoIds) return;
    let live = true;
    void Promise.all(
      filesRepoIds.split(",").map((id) =>
        listRepoFiles(id)
          .then((ps) => ps.map((path) => ({ repoId: id, projectId: filesProject, path })))
          .catch(() => []),
      ),
    ).then((lists) => live && setPaletteFiles(lists.flat()));
    return () => {
      live = false;
    };
  }, [paletteOpen, filesProject, filesRepoIds]);

  const paletteActions = useMemo((): PaletteItem[] => {
    const items: PaletteItem[] = [];
    const canNewTask = project ? projectRepos.length > 0 : ctx.repos.length > 0;
    if (canNewTask) items.push({ id: "new-task", kind: "Action", label: "New task", run: openNewTask });
    items.push({ id: "new-project", kind: "Action", label: "New project", run: () => setCreateProject(true) });
    if (project && canImport) {
      items.push({ id: "import", kind: "Action", label: "Import from Linear", run: () => setImportFor(project.id) });
    }
    items.push({ id: "settings", kind: "Action", label: "Open Settings", shortcut: "⌘,", run: () => go("settings") });
    items.push({ id: "onboarding", kind: "Action", label: "Show onboarding", run: () => setForceOnboarding(true) });
    for (const p of ctx.projects) {
      items.push({ id: `open-${p.id}`, kind: "Action", label: `Open ${p.name}`, keywords: p.key, run: () => go("board", p.id) });
    }
    if (work.blocked.length > 0) {
      items.push({
        id: "need-you",
        kind: "Action",
        label: `Review blocked tasks (${work.blocked.length})`,
        run: () => openTask(work.blocked[0]!.id),
      });
    }
    return items;
  }, [project, projectRepos.length, ctx.repos.length, ctx.projects, canImport, work.blocked, openNewTask, go, openTask]);

  const paletteSearch = (q: string): PaletteItem[] => {
    const keyOf = (pid: string, n: number) => taskKey(ctx.projectById.get(pid)?.key ?? "", n);
    const pool = work.tasks.filter((t) => {
      if (!q) return true;
      const hay = `${keyOf(t.projectId, t.number)} ${t.title} ${t.source?.identifier ?? ""}`.toLowerCase();
      return hay.includes(q);
    });
    const taskItems: PaletteItem[] = pool.slice(0, q ? 6 : 4).map((t) => ({
      id: `task-${t.id}`,
      kind: "Task",
      label: `${keyOf(t.projectId, t.number)} ${t.title}`,
      sub: ctx.projectById.get(t.projectId)?.name,
      run: () => openTask(t.id),
    }));
    const chatItems: PaletteItem[] = (allChats.data ?? [])
          .filter((c) => ctx.projectById.has(c.projectId) && (c.title || c.sessionTitle) && (!q || chatTitle(c).toLowerCase().includes(q)))
          .slice(0, q ? 5 : 4)
          .map((c) => ({
            id: `chat-${c.id}`,
            kind: "Chat",
            label: chatTitle(c),
            sub: ctx.projectById.get(c.projectId)?.name,
            run: () => openChatTab(c),
          }));
    // Files: name matches first (prefix, then anywhere), then path matches.
    const fileItems: PaletteItem[] = [];
    if (q) {
      const scored: { f: (typeof paletteFiles)[number]; s: number }[] = [];
      for (const f of paletteFiles) {
        const lower = f.path.toLowerCase();
        const name = lower.slice(lower.lastIndexOf("/") + 1);
        const s = name.startsWith(q) ? 0 : name.includes(q) ? 1 : lower.includes(q) ? 2 : -1;
        if (s >= 0) scored.push({ f, s });
        if (scored.length > 400) break;
      }
      scored.sort((a, b) => a.s - b.s || a.f.path.length - b.f.path.length);
      for (const { f } of scored.slice(0, 6)) {
        const slash = f.path.lastIndexOf("/");
        fileItems.push({
          id: `file-${f.repoId}-${f.path}`,
          kind: "File",
          label: f.path.slice(slash + 1),
          sub: `${ctx.repoById.get(f.repoId)?.name ?? ""}${slash > 0 ? ` / ${f.path.slice(0, slash)}` : ""}`,
          run: () => {
            tabs.openChat(fileTab(f.repoId, f.path));
            go("chat", f.projectId);
          },
        });
      }
    }
    return [...chatItems, ...fileItems, ...taskItems];
  };

  // ---- Screens outside the shell ----
  if (!ctx.loaded) return <div className="loading-screen" aria-busy="true">Loading…</div>;
  if (ctx.error && ctx.projects.length === 0) {
    return (
      <div className="loading-screen">
        <div className="center-state-body">
          <div className="state-icon-error" aria-hidden>
            !
          </div>
          <div className="center-state-title">Couldn't load your projects</div>
          <div className="center-state-text">{ctx.error}</div>
          <button type="button" className="btn" onClick={() => void ctx.refresh()}>
            Retry
          </button>
        </div>
      </div>
    );
  }
  if (showOnboarding) {
    return (
      <Onboarding
        onDone={(p) => {
          setForceOnboarding(false);
          go("board", p.id);
        }}
        onCancel={ctx.projects.length > 0 ? () => setForceOnboarding(false) : undefined}
        importingLegacy={legacy.busy}
        onImportLegacy={() =>
          void legacy.run().then((r) => {
            if (r && r.projects > 0) setForceOnboarding(false);
          })
        }
      />
    );
  }

  // ---- Content ----
  // The board handles its own "no repos" state; Tasks and Chat delegate it here.
  const noRepos = project !== null && projectRepos.length === 0 && route.page === "chat";
  let content: ReactNode;
  if (noRepos && project) {
    content = (
      <EmptyState
        title={`No repos in ${project.name}`}
        description={`${route.page === "chat" ? "A chat runs inside one of the project's repos." : "Every task belongs to one repo."} Add the root of a git checkout to start.`}
      >
        <button
          type="button"
          className="btn btn-primary"
          onClick={() => {
            openRepoSettings(project.id);
          }}
        >
          Add repo…
        </button>
      </EmptyState>
    );
  } else {
    switch (route.page) {
      case "board":
        content = (
          <BoardView
            projectId={route.projectId}
            onOpenTask={openTask}
            onNewProject={() => setCreateProject(true)}
            onOpenProjectSettings={openRepoSettings}
          />
        );
        break;
      case "chat": {
        const tab = tabs.chat;
        const tabPid = chatProjectOf(tab);
        const tabProject = tabPid ? ctx.projectById.get(tabPid) : undefined;
        const repos = tabProject ? ctx.reposOf(tabProject.id) : [];
        const openFile = tab ? parseFileTab(tab) : null;
        // The files panel follows the active tab: the open file's repo, or the chat's.
        const filesRepo =
          repos.find((r) => r.id === (openFile ? openFile.repoId : tab ? chatById.get(tab)?.repoId : null)) ?? repos[0];
        const items: TabItem[] = tabs.chats.map((t) => {
          const file = parseFileTab(t);
          if (file) {
            const name = file.path.split("/").pop() ?? file.path;
            if (file.commit) return { id: t, label: `Commit ${file.path.slice(0, 7)}`, closable: true, file: true };
            return { id: t, label: file.diff ? `${name} (changes)` : name, closable: true, file: true, dirty: dirtyTabs.has(t) };
          }
          const dp = draftProject(t);
          const c = dp ? undefined : chatById.get(t);
          const pid = dp ?? c?.projectId;
          return {
            id: t,
            label: c ? chatTitle(c) : "New chat",
            color: projectColor((pid && ctx.projectById.get(pid)?.color) || undefined),
            closable: true,
            live: liveChats.has(t),
            unread: unreadChats.has(t),
          };
        });
        content = (
          <div className="chat-tabs">
            <TabBar
              label="Open chats"
              items={items}
              active={tab}
              onPick={pickChatTab}
              onClose={closeChatTab}
              onCloseMany={(ids) => void closeChatTabs(ids)}
              after={
                tabProject && (
                  <button
                    type="button"
                    className="icon-btn tabbar-btn"
                    aria-label={`New chat in ${tabProject.name}`}
                    title={`New chat in ${tabProject.name}`}
                    onClick={() => newChat(tabProject.id)}
                  >
                    <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" aria-hidden focusable="false">
                      <path d="M6 2.5v7M2.5 6h7" />
                    </svg>
                  </button>
                )
              }
              end={
                filesRepo && (
                  <button
                    type="button"
                    className={`icon-btn tabbar-btn ${filesOpen ? "on" : ""}`}
                    aria-pressed={filesOpen}
                    aria-label="Project files"
                    title="Project files"
                    onClick={() => setFilesOpen((o) => !o)}
                  >
                    <svg width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" aria-hidden focusable="false">
                      <path d="M1.5 3.5A1 1 0 0 1 2.5 2.5h3l1.2 1.5h4.8a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z" />
                    </svg>
                  </button>
                )
              }
            />
            <div className="chat-body">
              {openFile && tabProject ? (
                ctx.repoById.get(openFile.repoId) && (
                  openFile.commit ? (
                    <CommitView key={tab} repo={ctx.repoById.get(openFile.repoId)!} worktree={openFile.worktree} sha={openFile.path} />
                  ) : openFile.diff ? (
                    <FileDiffView key={tab} repo={ctx.repoById.get(openFile.repoId)!} worktree={openFile.worktree} path={openFile.path} />
                  ) : (
                    <FileView key={tab} tab={tab!} repo={ctx.repoById.get(openFile.repoId)!} worktree={openFile.worktree} path={openFile.path} />
                  )
                )
              ) : tab && tabProject ? (
                <ChatView
                  key={tab}
                  project={tabProject}
                  repos={repos}
                  chatId={draftProject(tab) ? null : tab}
                  onCreated={(id) => tabs.replaceChat(tab, id)}
                  onShowChanges={() => {
                    setFilesMode("changes");
                    setFilesOpen(true);
                  }}
                  onOpenTask={openTask}
                  onOpenDiff={(repoId, path, worktree) => tabs.openChat(fileTab(repoId, path, true, worktree))}
                />
              ) : (
                <EmptyState title="No chat open" description="Pick a chat in the sidebar, or start one with the + next to a project." />
              )}
              {filesOpen && filesRepo && (
                <FilesPanel
                  key={filesRepo.id}
                  repo={filesRepo}
                  activePath={openFile && !openFile.commit && openFile.repoId === filesRepo.id && openFile.worktree === filesWorktree ? openFile.path : null}
                  activeCommit={openFile?.commit && openFile.repoId === filesRepo.id && openFile.worktree === filesWorktree ? openFile.path : null}
                  onOpenCommit={(sha) => tabs.openChat(commitTab(filesRepo.id, sha, filesWorktree))}
                  onAskChatForPr={
                    activeChatId
                      ? () =>
                          void sendToChat(
                            activeChatId,
                            "Open a pull request for this branch with what's already committed (don't commit anything new): push it and run " +
                              "`gh pr create` against the branch it split from. Follow .github/PULL_REQUEST_TEMPLATE.md if the repo has one. Don't merge it. Reply with the PR URL.",
                          ).catch((e: unknown) => toast("Couldn't ask the chat", String(e), "danger"))
                      : null
                  }
                  onOpenFile={(path) => tabs.openChat(fileTab(filesRepo.id, path, false, filesWorktree))}
                  onOpenDiff={(path) => tabs.openChat(fileTab(filesRepo.id, path, true, filesWorktree))}
                  worktree={filesWorktree}
                  onWorktree={setFilesWorktree}
                  chatId={activeChatId}
                  mode={filesMode}
                  onMode={setFilesMode}
                  onClose={() => setFilesOpen(false)}
                />
              )}
            </div>
          </div>
        );
        break;
      }
      case "project-settings":
        content = project && (
          <ProjectSettings
            project={project}
            section={nav.projectSection}
            onSection={nav.setProjectSection}
            onDeleted={() => go("board")}
            onOpenIntegrations={openIntegrations}
          />
        );
        break;
      case "overview":
        content = (
          <OverviewView
            chats={allChats.data ?? []}
            projects={ctx.projects}
            onOpen={openChatTab}
            onNewChat={newChat}
            onOpenProject={(pid) => {
              const latest = (allChats.data ?? []).filter((c) => c.projectId === pid).sort((a, b) => b.updatedAt - a.updatedAt)[0];
              if (latest) openChatTab(latest);
              else newChat(pid);
            }}
            onOpenBoard={(pid) => go("board", pid)}
            onNewProject={() => setCreateProject(true)}
          />
        );
        break;
      case "settings":
        content = <SettingsView section={nav.settingsSection} onSection={nav.setSettingsSection} updates={updates} />;
        break;
    }
  }

  const pageTitle = visible.page === "board" && !visible.projectId ? "All projects" : PAGE_TITLE[visible.page];
  const showNewTask =
    route.page === "board" && (project ? projectRepos.length > 0 : ctx.repos.length > 0);

  const foot: ProviderFoot =
    provider.connection === "connected"
      ? { tone: "ok", label: "Linear connected" }
      : provider.status?.hasKey
        ? { tone: "danger", label: "Linear unreachable" }
        : { tone: "muted", label: "No task manager connected" };

  return (
    <OpenChatContext.Provider value={openChatTab}>
    <div className="desk">
      <Sidebar
        mode={sideMode.current}
        projects={ctx.projects}
        chats={allChats.data ?? []}
        defaultRepo={(pid) => ctx.reposOf(pid)[0]?.id ?? null}
        activeChat={visible.page === "chat" ? tabs.chat : null}
        liveChats={liveChats}
        unreadChats={unreadChats}
        activeBoard={visible.page === "board" ? visible.projectId : undefined}
        settingsOpen={visible.page === "settings"}
        overviewOpen={visible.page === "overview"}
        onOverview={() => go("overview")}
        openTotal={work.openTotal}
        openByProject={openByProject}
        needYouByProject={needYouByProject}
        mappingDrift={mappingDrift}
        canNewTask={ctx.repos.length > 0}
        provider={foot}
        version={updates.version}
        updateAvailable={updates.available}
        updating={updates.installing}
        checkingUpdates={updates.checking}
        onUpdate={updates.check}
        onMode={goMode}
        onOpenChat={openChatTab}
        onNewChat={newChat}
        onDeleteChat={(c) => void removeChat(c)}
        onOpenBoard={openBoard}
        onNewTask={openNewTask}
        onProjectSettings={(pid, section = "general") => {
          setProjectSection(section);
          go("project-settings", pid);
        }}
        onSettings={() => go("settings")}
        onNewProject={() => setCreateProject(true)}
        onOpenPalette={() => setPaletteOpen(true)}
      />
      <div className="main-wrap">
        <main className="main">
          {route.page === "board" && (
            <TabBar
              label="Open boards"
              items={[
                { id: ALL_BOARD, label: "All projects", closable: false },
                ...tabs.boards.flatMap((id) => {
                  const bp = ctx.projectById.get(id);
                  return bp ? [{ id, label: bp.name, color: projectColor(bp.color), closable: true }] : [];
                }),
              ]}
              active={route.projectId ?? ALL_BOARD}
              onPick={(id) => openBoard(id === ALL_BOARD ? null : id)}
              onClose={closeBoardTab}
              onCloseMany={(ids) => ids.forEach(closeBoardTab)}
            />
          )}
          {route.page !== "chat" && <Topbar
            project={project}
            page={pageTitle}
            showImport={route.page === "board" && canImport}
            showNewTask={showNewTask}
            onImport={() => project && setImportFor(project.id)}
            onNewTask={openNewTask}
          />}
          {work.error && (
            <div className="banner banner-error" role="alert">
              Couldn't refresh tasks: {work.error}
            </div>
          )}
          <div className="content">{content}</div>

          {taskId && (
            <TaskPanel
              key={taskId}
              taskId={taskId}
              onClose={() => setTaskId(null)}
              onOpenTask={openTask}
              onOpenSources={(pid) => {
                nav.setProjectSection("sources");
                go("project-settings", pid);
              }}
            />
          )}
        </main>
      </div>

      {newTask && (
        <NewTaskDialog
          projectId={newTask.projectId}
          onClose={() => setNewTask(null)}
          onSaved={(t) => {
            setNewTask(null);
            openTask(t.id);
          }}
          onAddRepo={(pid) => {
            setNewTask(null);
            openRepoSettings(pid);
          }}
        />
      )}
      {importFor && (
        <ImportDialog
          projectId={importFor}
          projectName={ctx.projectById.get(importFor)?.name}
          onClose={() => setImportFor(null)}
          onImported={() => void invalidate("tasks")}
          onOpenSources={() => {
            const pid = importFor;
            setImportFor(null);
            nav.setProjectSection("sources");
            go("project-settings", pid);
          }}
        />
      )}
      {createProject && (
        <CreateProjectDialog
          onClose={() => setCreateProject(false)}
          onCreated={(p) => {
            setCreateProject(false);
            go("board", p.id);
          }}
        />
      )}
      {paletteOpen && (
        <CommandPalette actions={paletteActions} search={paletteSearch} onClose={() => setPaletteOpen(false)} />
      )}
    </div>
    </OpenChatContext.Provider>
  );
}
