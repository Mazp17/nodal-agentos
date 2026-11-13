// App navigation: a small route held in state (there are no URLs in a desktop app),
// persisted in `nodal.route` to return to where you were.

import { useCallback, useEffect, useState } from "react";
import { readJsonPref, writePref } from "./storage";

/** Project pages (and the global `board` with `projectId: null`). */
export type ProjectPage = "board" | "chat" | "project-settings";
export type Page = ProjectPage | "settings" | "overview";

/** `debug` exists only in dev builds. */
export type SettingsSection = "integrations" | "editor" | "chats" | "appearance" | "updates" | "diagnostics" | "debug";
export type ProjectSection = "general" | "repos" | "worktrees" | "sources" | "agents";

export interface Route {
  /** `null`: global view ("All projects", Settings). */
  projectId: string | null;
  page: Page;
}

/** These pages need a project; without one they fall back to the global `board`. */
const NEEDS_PROJECT: ReadonlySet<Page> = new Set(["chat", "project-settings"]);
// `tasks` is the old List page, now a view of the board; saved routes still carry it.
const PAGES: ReadonlySet<string> = new Set(["board", "chat", "tasks", "project-settings", "settings", "overview"]);
const HOME: Route = { projectId: null, page: "board" };

const isRoute = (v: unknown): v is Route => {
  if (!v || typeof v !== "object") return false;
  const r = v as Record<string, unknown>;
  return (r.projectId === null || typeof r.projectId === "string") && typeof r.page === "string" && PAGES.has(r.page);
};
const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");

const normalize = (r: Route): Route => {
  if ((r.page as string) === "tasks") return { projectId: r.projectId, page: "board" };
  if (r.page === "settings" || r.page === "overview") return { projectId: null, page: r.page };
  if (NEEDS_PROJECT.has(r.page) && !r.projectId) return { projectId: null, page: "board" };
  return r;
};

export interface Nav {
  route: Route;
  expanded: ReadonlySet<string>;
  settingsSection: SettingsSection;
  projectSection: ProjectSection;
  go: (page: Page, projectId?: string | null) => void;
  toggleProject: (projectId: string) => void;
  setExpanded: (projectId: string, open: boolean) => void;
  setSettingsSection: (s: SettingsSection) => void;
  setProjectSection: (s: ProjectSection) => void;
  /** If the route's project no longer exists, go back to "All projects". */
  forgetProject: (projectId: string) => void;
}

export function useNav(): Nav {
  // A saved `runs`/`run` page (removed) fails `isRoute` and falls back to HOME.
  const [route, setRoute] = useState<Route>(() => normalize(readJsonPref<Route>("route", HOME, isRoute)));
  const [expanded, setExpandedState] = useState<ReadonlySet<string>>(
    () => new Set(readJsonPref<string[]>("expandedProjects", [], isStringArray)),
  );
  const [settingsSection, setSettingsSection] = useState<SettingsSection>(() => {
    const s = readJsonPref<string>("settingsSection", "integrations", (v): v is string => typeof v === "string");
    if (s === "debug") return import.meta.env.DEV ? s : "integrations";
    return s === "editor" || s === "chats" || s === "appearance" || s === "updates" || s === "diagnostics" ? s : "integrations";
  });
  const [projectSection, setProjectSection] = useState<ProjectSection>("general");

  useEffect(() => {
    writePref("route", JSON.stringify(route));
  }, [route]);
  useEffect(() => writePref("expandedProjects", JSON.stringify([...expanded])), [expanded]);
  useEffect(() => writePref("settingsSection", JSON.stringify(settingsSection)), [settingsSection]);

  const go = useCallback((page: Page, projectId: string | null = null) => {
    const next = normalize({ projectId, page });
    setRoute(next);
    if (next.projectId) {
      const id = next.projectId;
      setExpandedState((s) => (s.has(id) ? s : new Set(s).add(id)));
    }
  }, []);

  const setExpanded = useCallback((id: string, open: boolean) => {
    setExpandedState((s) => {
      if (s.has(id) === open) return s;
      const n = new Set(s);
      if (open) n.add(id);
      else n.delete(id);
      return n;
    });
  }, []);
  const toggleProject = useCallback(
    (id: string) =>
      setExpandedState((s) => {
        const n = new Set(s);
        if (n.has(id)) n.delete(id);
        else n.add(id);
        return n;
      }),
    [],
  );

  const forgetProject = useCallback((id: string) => {
    setRoute((cur) => (cur.projectId === id ? HOME : cur));
    setExpandedState((s) => {
      if (!s.has(id)) return s;
      const n = new Set(s);
      n.delete(id);
      return n;
    });
  }, []);

  return {
    route,
    expanded,
    settingsSection,
    projectSection,
    go,
    toggleProject,
    setExpanded,
    setSettingsSection,
    setProjectSection,
    forgetProject,
  };
}

export const PAGE_TITLE: Record<Page, string> = {
  board: "Tasks",
  chat: "Chat",
  "project-settings": "Settings",
  settings: "Settings",
  overview: "Overview",
};
