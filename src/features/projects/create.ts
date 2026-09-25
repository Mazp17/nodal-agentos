// Creates a project with its repos and, optionally, a Linear source. Used by
// onboarding and the "New project" dialog.

import { addRepo, createProject, createSourceLink } from "../../domain/api";
import { invalidateProviders } from "../../domain/hooks/providers";
import { suggestProjectKey, type ProjectsState } from "../../domain/hooks/projects";
import type { Project, ScopeRef } from "../../domain/types";
const LINEAR = "linear";

/** Same palette as the backend (`validate::PALETTE`), in the same order. */
export const PROJECT_COLORS = [
  "oklch(0.74 0.15 55)",
  "oklch(0.72 0.13 250)",
  "oklch(0.74 0.12 150)",
  "oklch(0.70 0.15 320)",
  "oklch(0.78 0.13 88)",
  "oklch(0.68 0.16 25)",
  "oklch(0.72 0.10 200)",
  "oklch(0.70 0.12 290)",
] as const;

export interface ProjectDraft {
  name: string;
  key: string;
  color: string;
  /** Optional; empty = no description. */
  description?: string;
  /** Already-resolved git roots. */
  repos: string[];
  /** Linear team or project to connect, if any. */
  scope: ScopeRef | null;
  /** The key was suggested by Nodal: if it collides (e.g. with an archived project), try another. */
  autoKey?: boolean;
}

export interface CreateOutcome {
  project: Project;
  reposAdded: number;
  sourceConnected: boolean;
  /** Steps that failed after creating the project (repos, source). */
  failures: string[];
}

/**
 * Creates the project, then each repo and the source. If a repo or the source fails, the
 * project stays created and the error is returned for display (they can be added from Settings).
 */
export async function createProjectWithRepos(ctx: ProjectsState, d: ProjectDraft): Promise<CreateOutcome> {
  let key = d.key.trim().toUpperCase();
  const tried = [key];
  let project: Project;
  for (;;) {
    try {
      // Straight to the API: reloads only once at the end. Reloading now would make the shell see
      // a project and unmount onboarding with the repos still to be added.
      project = await createProject({ name: d.name.trim(), key, color: d.color, description: d.description?.trim() || null });
      break;
    } catch (e) {
      if (!d.autoKey || !String(e).includes("already used") || tried.length >= 20) throw e;
      key = suggestProjectKey(d.name, [...ctx.projects.map((p) => p.key), ...tried]);
      tried.push(key);
    }
  }
  const failures: string[] = [];
  const repoIds: string[] = [];
  for (const path of d.repos) {
    try {
      repoIds.push((await addRepo(project.id, { path })).id);
    } catch (e) {
      failures.push(`${path}: ${String(e)}`);
    }
  }
  let sourceConnected = false;
  if (d.scope) {
    try {
      await createSourceLink({ projectId: project.id, provider: LINEAR, scope: d.scope, defaultRepoId: repoIds[0] ?? null });
      sourceConnected = true;
      invalidateProviders("links");
    } catch (e) {
      failures.push(`Linear · ${d.scope.name}: ${String(e)}`);
    }
  }
  await ctx.refresh();
  return { project, reposAdded: repoIds.length, sourceConnected, failures };
}

/** "Payments · 2 repos · Linear connected" */
export const createdSummary = (name: string, repos: number, source: boolean) =>
  [name, `${repos} repo${repos === 1 ? "" : "s"}`, ...(source ? ["Linear connected"] : [])].join(" · ");
