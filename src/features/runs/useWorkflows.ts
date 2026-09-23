import { useEffect, useState } from "react";
import { listWorkflows } from "./api";
import type { WorkflowInfo } from "./types";

const LAST_WORKFLOW_KEY = "agent-desk.lastWorkflow";
export const DEFAULT_WORKFLOW = "linear-issue";

export function readLastWorkflow(): string {
  try {
    return localStorage.getItem(LAST_WORKFLOW_KEY) ?? DEFAULT_WORKFLOW;
  } catch {
    return DEFAULT_WORKFLOW;
  }
}
export function writeLastWorkflow(v: string) {
  try {
    localStorage.setItem(LAST_WORKFLOW_KEY, v);
  } catch {
    /* ignorar */
  }
}

/** Clave "" = catálogo global (repoPath null). */
export type WorkflowCatalogs = Record<string, WorkflowInfo[]>;

/** Catálogo global más el de cada repo mapeado; cada uno se pide una sola vez. */
export function useWorkflows(repoPaths: string[]): WorkflowCatalogs {
  const [catalogs, setCatalogs] = useState<WorkflowCatalogs>({});
  const key = [...new Set(repoPaths)].sort().join("\n");

  useEffect(() => {
    let cancelled = false;
    const wanted = ["", ...(key ? key.split("\n") : [])];
    for (const repo of wanted) {
      if (catalogs[repo]) continue;
      listWorkflows(repo || null)
        .then((list) => {
          if (!cancelled) setCatalogs((prev) => ({ ...prev, [repo]: list }));
        })
        .catch((e) => console.error("list_workflows", repo, e));
    }
    return () => {
      cancelled = true;
    };
    // Solo cuando cambian los repos; `catalogs` se consulta para no repedir.
  }, [key]);

  return catalogs;
}

/** Workflow a usar: la elección si existe en el catálogo; si no, el primero. */
export function pickWorkflow(catalog: WorkflowInfo[], wanted: string): string {
  if (catalog.length === 0 || catalog.some((w) => w.name === wanted)) return wanted;
  return catalog[0].name;
}
