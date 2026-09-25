// Projects and repos for the whole app (sidebar, settings, dialogs, views), on top of the
// shared stores in `store.ts`: no queries of their own. This module's mutations
// invalidate on their own; other features can call `refresh()` after changing something.

import { useMemo } from "react";
import * as api from "../api";
import type { Project, Repo } from "../types";
import { invalidate, KEYS, setData, useProjectList, useRepoList } from "./store";

export interface ProjectsState {
  projects: Project[];
  repos: Repo[];
  /** `false` until the first response (good or bad). */
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

const refresh = () => invalidate("projects", "repos");

/** Awaits the re-read: whoever creates something sees the already-updated state when continuing. */
const after = <T,>(p: Promise<T>): Promise<T> =>
  p.then(
    async (v) => {
      await refresh();
      return v;
    },
    async (e: unknown) => {
      await refresh();
      throw e;
    },
  );

const MUTATIONS = {
  refresh,
  createProject: (input: api.NewProject) => after(api.createProject(input)),
  // Optimistic: segmented controls and colors respond instantly; the re-read corrects it if it failed.
  updateProject: (id: string, patch: api.ProjectPatch) => {
    setData<Project[]>(KEYS.projects, (ps) => ps.map((p) => (p.id === id ? ({ ...p, ...patch } as Project) : p)));
    return after(api.updateProject(id, patch));
  },
  deleteProject: (id: string) => after(api.deleteProject(id)),
  addRepo: (projectId: string, input: api.NewRepo) => after(api.addRepo(projectId, input)),
  updateRepo: (id: string, patch: api.RepoPatch) => {
    setData<Repo[]>(KEYS.repos, (rs) => rs.map((r) => (r.id === id ? ({ ...r, ...patch } as Repo) : r)));
    return after(api.updateRepo(id, patch));
  },
  deleteRepo: (id: string) => after(api.deleteRepo(id)),
};

const NONE: never[] = [];

export function useProjects(): ProjectsState {
  const p = useProjectList();
  const r = useRepoList();
  const loaded = (p.data !== undefined || p.error !== null) && (r.data !== undefined || r.error !== null);
  const error = p.error ?? r.error;
  const projects = p.data ?? NONE;
  const repos = r.data ?? NONE;
  return useMemo((): ProjectsState => {
    const byProject = new Map<string, Repo[]>();
    for (const x of repos) byProject.set(x.projectId, [...(byProject.get(x.projectId) ?? []), x]);
    return {
      projects,
      repos,
      loaded,
      error,
      projectById: new Map(projects.map((x) => [x.id, x])),
      repoById: new Map(repos.map((x) => [x.id, x])),
      reposOf: (projectId) => byProject.get(projectId) ?? NONE,
      ...MUTATIONS,
    };
  }, [projects, repos, loaded, error]);
}

/** Last folder of the path (`/a/b/repo` → `repo`). */
export const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/**
 * Project key from the name: 3 uppercase letters ("Payments" → "PAY"),
 * with a digit if it clashes with an existing one. Satisfies `validate::project_key` (2-6, starts
 * with a letter).
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
