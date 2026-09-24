// Proyectos y repos compartidos por toda la app (sidebar, settings, diálogos, vistas).
// Un solo `list_projects` + `list_repos` por refresh; las mutaciones de este módulo
// refrescan solas. Otras features pueden llamar `refresh()` tras cambiar algo.

import { createContext, createElement, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import * as api from "../api";
import type { Project, Repo } from "../types";

export interface ProjectsState {
  projects: Project[];
  repos: Repo[];
  /** `false` hasta la primera respuesta (buena o mala). */
  loaded: boolean;
  error: string | null;
  projectById: Map<string, Project>;
  repoById: Map<string, Repo>;
  reposOf: (projectId: string) => Repo[];
  refresh: () => Promise<void>;
  createProject: (input: api.NewProject) => Promise<Project>;
  updateProject: (id: string, patch: api.ProjectPatch) => Promise<Project>;
  deleteProject: (id: string) => Promise<void>;
  addRepo: (projectId: string, input: api.NewRepo) => Promise<Repo>;
  updateRepo: (id: string, patch: api.RepoPatch) => Promise<Repo>;
  deleteRepo: (id: string) => Promise<void>;
}

const Ctx = createContext<ProjectsState | null>(null);

/** Refresco periódico: los imports y la migración pueden crear proyectos por fuera. */
const POLL_MS = 30_000;

export function ProjectsProvider({ children }: { children: ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([]);
  const [repos, setRepos] = useState<Repo[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const id = ++seq.current;
    try {
      const [ps, rs] = await Promise.all([api.listProjects(), api.listRepos(null)]);
      if (id !== seq.current) return;
      setProjects(ps);
      setRepos(rs);
      setError(null);
    } catch (e) {
      if (id === seq.current) setError(String(e));
    } finally {
      if (id === seq.current) setLoaded(true);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const t = setInterval(() => void refresh(), POLL_MS);
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  const value = useMemo((): ProjectsState => {
    const after = <T,>(p: Promise<T>) =>
      p.then(
        (v) => {
          void refresh();
          return v;
        },
        (e: unknown) => {
          void refresh();
          throw e;
        },
      );
    const sorted = [...repos].sort((a, b) => a.position - b.position || a.name.localeCompare(b.name));
    return {
      projects,
      repos: sorted,
      loaded,
      error,
      projectById: new Map(projects.map((p) => [p.id, p])),
      repoById: new Map(repos.map((r) => [r.id, r])),
      reposOf: (projectId) => sorted.filter((r) => r.projectId === projectId),
      refresh,
      createProject: (input) => after(api.createProject(input)),
      updateProject: (id, patch) => after(api.updateProject(id, patch)),
      deleteProject: (id) => after(api.deleteProject(id)),
      addRepo: (projectId, input) => after(api.addRepo(projectId, input)),
      updateRepo: (id, patch) => after(api.updateRepo(id, patch)),
      deleteRepo: (id) => after(api.deleteRepo(id)),
    };
  }, [projects, repos, loaded, error, refresh]);

  return createElement(Ctx.Provider, { value }, children);
}

export function useProjects(): ProjectsState {
  const v = useContext(Ctx);
  if (!v) throw new Error("useProjects must be used inside <ProjectsProvider>");
  return v;
}

/** Última carpeta del path (`/a/b/repo` → `repo`). */
export const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/**
 * Key de proyecto a partir del nombre: 3 letras en mayúscula ("Payments" → "PAY"),
 * con un dígito si choca con una existente. Cumple `validate::project_key` (2-6, empieza
 * con letra).
 */
export function suggestProjectKey(name: string, taken: Iterable<string>): string {
  const used = new Set([...taken].map((k) => k.toUpperCase()));
  const letters = name
    .normalize("NFD")
    .replace(/[^A-Za-z]/g, "")
    .toUpperCase();
  let base = letters.slice(0, 3);
  if (base.length < 2) base = "PRJ";
  if (!used.has(base)) return base;
  for (let n = 2; n < 100; n++) {
    const k = `${base}${n}`;
    if (!used.has(k)) return k;
  }
  return base;
}

export const isValidProjectKey = (k: string) => /^[A-Z][A-Z0-9]{1,5}$/.test(k.trim().toUpperCase());
