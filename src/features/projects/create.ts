// Alta de un proyecto con sus repos y, opcionalmente, una fuente de Linear. La usan el
// onboarding y el diálogo "New project".

import { addRepo, createSourceLink } from "../../domain/api";
import { invalidateProviders } from "../../domain/hooks/providers";
import { suggestProjectKey, type ProjectsState } from "../../domain/hooks/projects";
import type { Project, ScopeRef } from "../../domain/types";
const LINEAR = "linear";

/** Misma paleta que el backend (`validate::PALETTE`), en el mismo orden. */
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
  /** Raíces git ya resueltas. */
  repos: string[];
  /** Team o project de Linear a conectar, si hay. */
  scope: ScopeRef | null;
  /** La key la sugirió Nodal: si choca (p. ej. con un proyecto archivado), prueba otra. */
  autoKey?: boolean;
}

export interface CreateOutcome {
  project: Project;
  reposAdded: number;
  sourceConnected: boolean;
  /** Pasos que fallaron después de crear el proyecto (repos, fuente). */
  failures: string[];
}

/**
 * Crea el proyecto y después cada repo y la fuente. Si falla un repo o la fuente, el
 * proyecto queda creado y el error se devuelve para mostrarlo (se agregan desde Settings).
 */
export async function createProjectWithRepos(ctx: ProjectsState, d: ProjectDraft): Promise<CreateOutcome> {
  let key = d.key.trim().toUpperCase();
  const tried = [key];
  let project: Project;
  for (;;) {
    try {
      project = await ctx.createProject({ name: d.name.trim(), key, color: d.color });
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
