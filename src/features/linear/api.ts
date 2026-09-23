import { invoke } from "@tauri-apps/api/core";

// Espejo de src-tauri/src/linear/model.rs y src-tauri/src/config/mod.rs.

export type StateType = "triage" | "backlog" | "unstarted" | "started" | "completed" | "canceled";

export interface Viewer { name: string; email: string }
export interface Team { id: string; key: string; name: string }
export interface WorkflowState {
  id: string;
  name: string;
  type: StateType | string;
  position: number;
  color: string;
}
export interface Issue {
  id: string;
  identifier: string;
  title: string;
  url: string;
  /** 0 = sin prioridad, 1 = urgente … 4 = baja */
  priority: number;
  priorityLabel: string;
  team: Team;
  state: WorkflowState;
  assignee: { id: string; name: string; displayName: string } | null;
  project: { id: string; name: string } | null;
  parent: { identifier: string } | null;
  updatedAt: string;
}
export interface Board {
  teams: { team: Team; states: WorkflowState[] }[];
  issues: Issue[];
  truncated: boolean;
}

export interface RepoMapping { teamId?: string; projectId?: string; path: string }
export interface AppConfig { repos: RepoMapping[]; concurrency: number }

export type LinearErrorKind =
  | "missingKey"
  | "invalidKey"
  | "network"
  | "rateLimited"
  | "keychain"
  | "api"
  | "unknown";
export interface LinearError { kind: LinearErrorKind; message: string }

/** Los comandos de Linear rechazan con `{kind, message}`; cualquier otra cosa se normaliza. */
export function toLinearError(err: unknown): LinearError {
  if (err && typeof err === "object" && "kind" in err && "message" in err) {
    return err as LinearError;
  }
  return { kind: "unknown", message: String(err) };
}

export const linearApi = {
  keyStatus: () => invoke<{ configured: boolean }>("linear_key_status"),
  /** Valida contra Linear antes de guardar en el llavero; la key no vuelve nunca. */
  setApiKey: (key: string) => invoke<Viewer>("linear_set_api_key", { key }),
  clearApiKey: () => invoke<void>("linear_clear_api_key"),
  viewer: () => invoke<Viewer>("linear_viewer"),
  teams: () => invoke<Team[]>("linear_teams"),
  board: (teamIds: string[] | null) => invoke<Board>("linear_board", { teamIds }),
};

export const configApi = {
  get: () => invoke<AppConfig>("get_config"),
  /** Rechaza con un string (errores de validación, uno por línea). */
  save: (config: AppConfig) => invoke<AppConfig>("save_config", { config }),
};

/**
 * Mismo criterio que `config::resolve` en Rust (team+proyecto > proyecto > team),
 * duplicado acá para pintar el indicador de repo en cada card sin un invoke por card.
 */
export function resolveRepo(config: AppConfig, teamId: string, projectId?: string | null): string | null {
  const find = (team?: string, project?: string) =>
    config.repos.find((m) => m.teamId === team && m.projectId === project)?.path;
  const pid = projectId ?? undefined;
  return (pid && (find(teamId, pid) ?? find(undefined, pid))) || find(teamId, undefined) || null;
}
