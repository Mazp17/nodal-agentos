// Alta de un proyecto con sus repos y, opcionalmente, una fuente de Linear. La usan el
// onboarding y el diálogo "New project".

import { addRepo, createSourceLink } from "../../domain/api";
import type { ProjectsState } from "../../domain/hooks/projects";
import type { Project, ScopeRef } from "../../domain/types";
import { LINEAR } from "../../shell/providerStatus";

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
  const project = await ctx.createProject({ name: d.name.trim(), key: d.key.trim().toUpperCase(), color: d.color });
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
