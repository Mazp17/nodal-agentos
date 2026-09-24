// Navegación de la app: una ruta chica en estado (no hay URLs en una app de escritorio),
// persistida en `nodal.route` para volver a donde estabas.

import { useCallback, useEffect, useState } from "react";
import { readJsonPref, writePref } from "./storage";

/** Páginas de un proyecto (y las globales `board`/`runs` con `projectId: null`). */
export type ProjectPage = "board" | "tasks" | "runs" | "activity" | "project-settings";
export type Page = ProjectPage | "settings" | "run";

export type SettingsSection = "integrations" | "execution" | "updates" | "diagnostics";
export type ProjectSection = "general" | "repos" | "sources";

export interface Route {
  /** `null`: vista global ("All projects", Runs, Settings). */
  projectId: string | null;
  page: Page;
  /** Solo en `page: "run"`. */
  runId?: string;
}

/** Estas páginas necesitan proyecto; sin proyecto se cae a `board` global. */
const NEEDS_PROJECT: ReadonlySet<Page> = new Set(["tasks", "activity", "project-settings"]);
const PAGES: ReadonlySet<string> = new Set(["board", "tasks", "runs", "activity", "project-settings", "settings", "run"]);
const HOME: Route = { projectId: null, page: "board" };

const isRoute = (v: unknown): v is Route => {
  if (!v || typeof v !== "object") return false;
  const r = v as Record<string, unknown>;
  return (r.projectId === null || typeof r.projectId === "string") && typeof r.page === "string" && PAGES.has(r.page);
};
const isStringArray = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");

const normalize = (r: Route): Route => {
  if (r.page === "run" && !r.runId) return HOME;
  if (r.page === "settings") return { projectId: null, page: "settings" };
  if (NEEDS_PROJECT.has(r.page) && !r.projectId) return { projectId: null, page: "board" };
  return r;
};

export interface Nav {
  route: Route;
  /** De dónde se abrió el run (para "Back" y Esc). */
  runFrom: Route;
  expanded: ReadonlySet<string>;
  settingsSection: SettingsSection;
  projectSection: ProjectSection;
  go: (page: Page, projectId?: string | null) => void;
  openRun: (runId: string) => void;
  back: () => void;
  toggleProject: (projectId: string) => void;
  setExpanded: (projectId: string, open: boolean) => void;
  setSettingsSection: (s: SettingsSection) => void;
  setProjectSection: (s: ProjectSection) => void;
  /** Si el proyecto de la ruta ya no existe, vuelve a "All projects". */
  forgetProject: (projectId: string) => void;
}

export function useNav(): Nav {
  const [route, setRoute] = useState<Route>(() => {
    const saved = readJsonPref<Route>("route", HOME, isRoute);
    // Un run abierto no se restaura: su contexto (de dónde vino) se perdió.
    return saved.page === "run" ? HOME : normalize(saved);
  });
  const [runFrom, setRunFrom] = useState<Route>(HOME);
  const [expanded, setExpandedState] = useState<ReadonlySet<string>>(
    () => new Set(readJsonPref<string[]>("expandedProjects", [], isStringArray)),
  );
  const [settingsSection, setSettingsSection] = useState<SettingsSection>(() => {
    const s = readJsonPref<string>("settingsSection", "integrations", (v): v is string => typeof v === "string");
    return s === "execution" || s === "updates" || s === "diagnostics" ? s : "integrations";
  });
  const [projectSection, setProjectSection] = useState<ProjectSection>("general");

  useEffect(() => {
    if (route.page !== "run") writePref("route", JSON.stringify(route));
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

  const openRun = useCallback(
    (runId: string) => {
      if (route.page !== "run") setRunFrom(route);
      setRoute({ projectId: route.projectId, page: "run", runId });
    },
    [route],
  );

  const back = useCallback(() => setRoute((cur) => (cur.page === "run" ? runFrom : cur)), [runFrom]);

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
    setRunFrom((cur) => (cur.projectId === id ? HOME : cur));
    setExpandedState((s) => {
      if (!s.has(id)) return s;
      const n = new Set(s);
      n.delete(id);
      return n;
    });
  }, []);

  return {
    route,
    runFrom,
    expanded,
    settingsSection,
    projectSection,
    go,
    openRun,
    back,
    toggleProject,
    setExpanded,
    setSettingsSection,
    setProjectSection,
    forgetProject,
  };
}

export const PAGE_TITLE: Record<Page, string> = {
  board: "Board",
  tasks: "Tasks",
  runs: "Runs",
  activity: "Activity",
  "project-settings": "Settings",
  settings: "Settings",
  run: "Run",
};
