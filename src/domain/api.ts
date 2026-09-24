// Firmas de los comandos de F1. El backend todavía no los expone: cada uno lleva un
// `TODO(F1-X)` con el agente que lo implementa (A: migración y rebrand, B: cola, CRUD y
// runs, C: proveedores). No usar desde la UI hasta que existan.
//
// Todos rechazan con un string (en inglés) listo para mostrar. Tauri pasa los argumentos
// camelCase a los parámetros snake_case del comando.
//
// Los DTOs de este archivo (entradas y reportes) son contrato propuesto: el agente que
// implementa el comando crea el struct Rust equivalente con serde camelCase.
//
// Patches: campo ausente = no tocar; `null` = borrar. En Rust eso necesita distinguir
// "falta" de "null" (`Option<Option<T>>` con `#[serde(default, deserialize_with = ...)]`
// o `serde_with::double_option`): un `Option<Option<T>>` pelado lee `null` como "falta".

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AgentSource,
  ExtKind,
  Executor,
  ExternalState,
  Finish,
  Isolation,
  LaunchOptions,
  Priority,
  Project,
  RelationKind,
  Repo,
  RepoRule,
  Run,
  RunLight,
  ScopeRef,
  Settings,
  SourceLink,
  StateMap,
  Task,
  TaskRelation,
  TaskStatus,
} from "./types";
import type { Transcript } from "../features/runs/types";

// ---------- DTOs ----------

/**
 * Confianza de Claude Code en una carpeta (solo lectura de `~/.claude.json`):
 * `repo` = entrada de la raíz canónica del repo (el principal, para worktrees); `parent` =
 * la carpeta o un padre dentro del repo; `notTrusted` = va a mostrar el diálogo; `unknown`
 * (con `trusted: null`) = sin config legible.
 */
export interface RepoTrust {
  trusted: boolean | null;
  source: "repo" | "parent" | "notTrusted" | "unknown";
  matchedPath: string | null;
}

/**
 * `running`/`capacity`: slots ocupados de la concurrencia global (regla del pump). `needYou`:
 * tareas Blocked + runs migrados sin confirmar + sesiones esperando permiso/input, sin contar
 * dos veces la misma tarea. `queued`: listos para salir.
 */
export interface WorkSummary {
  running: number;
  capacity: number;
  needYou: number;
  queued: number;
}

/** Sesiones vivas trabajando/esperando y subagentes activos (mismo criterio que `activity_summary`). */
export interface RepoActivityCount {
  repoId: string;
  repoPath: string;
  sessions: number;
  agents: number;
}

/** Totales sin contar dos veces lo que cae en repos anidados. */
export interface ProjectActivity {
  projectId: string;
  repos: RepoActivityCount[];
  sessions: number;
  agents: number;
  generatedAt: number;
}

/** `exists`: la carpeta es un worktree vivo. `ahead`: commits fuera de la base; `unpushed`: además fuera de todo remoto. */
export interface WorktreeStatus {
  exists: boolean;
  branch: string | null;
  base: string | null;
  ahead: number;
  unpushed: number;
  dirty: boolean;
}

/**
 * `key`: 2-6 mayúsculas/dígitos, único. `color`: si falta, se asigna uno de la paleta.
 * Cambiar la key renumera los ids visibles (`PAY-1` → `WEB-1`) pero no renombra ramas ni
 * worktrees ya creados.
 */
export interface NewProject {
  name: string;
  key: string;
  color?: string;
  description?: string | null;
}

/** Solo los campos presentes se cambian; `null` borra los opcionales. */
export type ProjectPatch = Partial<Pick<Project, "name" | "key" | "color" | "description" | "defaultExecutor" | "reviewer">> & {
  archived?: boolean;
};

/** `path` puede ser cualquier carpeta dentro del repo: se resuelve a la raíz git. */
export interface NewRepo extends LaunchOptions {
  path: string;
  name?: string;
  defaultExecutor?: Executor | null;
  defaultIsolation?: Isolation;
  defaultFinish?: Finish;
  defaultReview?: boolean;
  reviewer?: string | null;
}

export type RepoPatch = Partial<Omit<NewRepo, "path" | "model" | "effort" | "permissionMode">> & {
  model?: string | null;
  effort?: string | null;
  permissionMode?: string | null;
  position?: number;
};

/** Lo que manda la UI para el plan; el backend lo guarda como `PlanRef`. */
export type PlanInput = { kind: "text"; text: string } | { kind: "file"; path: string };

export interface NewTask {
  projectId: string;
  repoId: string;
  title: string;
  plan: PlanInput;
  status?: TaskStatus;
  priority?: Priority;
  labels?: string[];
  acceptance?: string[];
  assignee?: Executor | null;
  isolation?: Isolation | null;
  finish?: Finish | null;
  review?: boolean | null;
}

/**
 * `repoId` mueve la tarea a otro repo del mismo proyecto (sin runs en curso ni worktree; un
 * plan `.md` del repo viejo exige mandar otro `plan`).
 * En las importadas solo se acepta `repoId`, `plan` (queda `planOverridden`), `status`,
 * `acceptance` y las opciones de ejecución.
 */
export type TaskPatch = Partial<Omit<NewTask, "projectId">>;

export interface ExecutorInfo {
  executor: Executor;
  /** Del frontmatter (`description`) o del `meta` del workflow. */
  description: string | null;
  /** Agentes: `tools` del frontmatter. */
  tools: string[] | null;
  /** Workflows: sincronizan el proveedor por su cuenta (`meta.managesSource`). */
  managesSource: string | null;
  /** Workflows: revisan por su cuenta (`meta.reviews`); se saltan el gate. */
  reviews: boolean;
  /** Archivo de la definición, para agentes y workflows. */
  path: string | null;
  source: AgentSource | null;
}

/** Overrides para este run; lo que falte sale de la tarea → repo → proyecto. */
export interface LaunchInput {
  executor?: Executor;
  isolation?: Isolation;
  finish?: Finish;
  review?: boolean;
  extraInstructions?: string;
  options?: LaunchOptions;
}

export type DiffFileStatus = "added" | "modified" | "deleted" | "renamed";

export interface DiffLine {
  kind: "context" | "add" | "del";
  text: string;
  oldNo: number | null;
  newNo: number | null;
}

export interface DiffHunk {
  header: string;
  oldStart: number;
  oldLines: number;
  newStart: number;
  newLines: number;
  lines: DiffLine[];
}

export interface FileDiff {
  path: string;
  oldPath: string | null;
  status: DiffFileStatus;
  additions: number;
  deletions: number;
  binary: boolean;
  hunks: DiffHunk[];
}

export interface CommitInfo {
  sha: string;
  shortSha: string;
  subject: string;
  author: string;
  /** Fecha del autor, epoch ms. */
  at: number;
}

export interface RunDiff {
  /** Ref base (`git diff <base>...HEAD`). */
  base: string;
  /** Rama del run; `null` con HEAD desacoplado. */
  branch: string | null;
  /** Commits de la rama que no están en `base`, del más nuevo al más viejo (hasta 200). */
  commits: CommitInfo[];
  /** El run sigue activo: el diff puede cambiar. */
  live: boolean;
  cwd: string;
  /** Incluye cambios sin commitear (el run sigue activo). */
  includesWorkingTree: boolean;
  files: FileDiff[];
  /** Patch completo, para "Copy patch". */
  patch: string;
}

export interface ProviderStatus {
  provider: string;
  hasKey: boolean;
  /** Nombre del usuario si la key es válida. */
  viewer: string | null;
  error: string | null;
  /** Últimos 4 caracteres de la key guardada (nunca la key); `null` sin key o si es corta. */
  keyHint: string | null;
}

export interface ImportableItem {
  externalId: string;
  identifier: string;
  title: string;
  url: string;
  state: ExternalState;
  labels: string[];
  /** Por `repoRules` o `defaultRepoId`. */
  suggestedRepoId: string | null;
  /** Si ya está importada, su tarea. */
  taskId: string | null;
}

export interface ImportResult {
  imported: Task[];
  skipped: { externalId: string; reason: string }[];
}

export interface SyncReport {
  /** Tareas refrescadas desde el proveedor. */
  pulled: number;
  /** Escrituras en el proveedor (estados y comentarios). */
  pushed: number;
  /** Tareas nuevas por auto-import. */
  imported: number;
  errors: string[];
  /** Avisos: estados nuevos/desaparecidos, ítems sin repo para el auto-import, pushes descartados. */
  notices: string[];
}

export type MapOrigin = "suggested" | "confirmed" | "unmapped";

export interface SourceStatesReport {
  /** Estados actuales del proveedor para el scope. */
  states: ExternalState[];
  /** Mapeo guardado completado con la propuesta de Nodal para lo que falte. */
  proposal: StateMap;
  pullOrigin: Record<string, MapOrigin>;
  pushOrigin: Partial<Record<TaskStatus, MapOrigin>>;
  /** Contra `knownStates`: estados nuevos y desaparecidos. */
  added: ExternalState[];
  removed: ExternalState[];
}

export interface NewSourceLink {
  projectId: string;
  provider: string;
  scope: ScopeRef;
  defaultRepoId?: string | null;
  repoRules?: RepoRule[];
  autoImport?: boolean;
}

export type SourceLinkPatch = Partial<Pick<SourceLink, "defaultRepoId" | "repoRules" | "autoImport">>;

export interface LegacyImportReport {
  /** Copia intacta de la carpeta elegida (`legacy-backup-<ts>/`). */
  backupDir: string;
  projects: number;
  repos: number;
  tasks: number;
  runs: number;
  /** Ya importados antes (idempotencia). */
  alreadyImported: number;
  /** Archivos o registros salteados, con el motivo. */
  skipped: string[];
}

// ---------- Proyectos ----------

export const listProjects = (includeArchived = false) => invoke<Project[]>("list_projects", { includeArchived });
export const createProject = (input: NewProject) => invoke<Project>("create_project", { input });
export const updateProject = (id: string, patch: ProjectPatch) => invoke<Project>("update_project", { id, patch });
/** Borra en cascada repos, tareas y fuentes. */
export const deleteProject = (id: string) => invoke<void>("delete_project", { id });

// ---------- Repos ----------

/** `null`: todos los proyectos. */
export const listRepos = (projectId: string | null) => invoke<Repo[]>("list_repos", { projectId });
/** Rechaza si no está en un repo git o si ya está en otro proyecto. */
export const addRepo = (projectId: string, input: NewRepo) => invoke<Repo>("add_repo", { projectId, input });
export const updateRepo = (id: string, patch: RepoPatch) => invoke<Repo>("update_repo", { id, patch });
/** Rechaza si el repo tiene tareas. */
export const deleteRepo = (id: string) => invoke<void>("delete_repo", { id });

// ---------- Tareas ----------

/** `null`: todas. */
export const listTasks = (projectId: string | null) => invoke<Task[]>("list_tasks", { projectId });
export const getTask = (id: string) => invoke<Task>("get_task", { id });
export const createTask = (input: NewTask) => invoke<Task>("create_task", { input });
export const updateTask = (id: string, patch: TaskPatch) => invoke<Task>("update_task", { id, patch });
export const deleteTask = (id: string) => invoke<void>("delete_task", { id });
/** Arrastre en el board: cambia de columna y/o posición. */
export const moveTask = (id: string, status: TaskStatus, position: number) =>
  invoke<Task>("move_task", { id, status, position });
/**
 * Nuevo orden de una columna: renumera posiciones en una transacción. Todas las tareas deben
 * estar en `status` y en el mismo proyecto; las de la columna que falten quedan detrás.
 */
export const reorderTasks = (status: TaskStatus, orderedIds: string[]) =>
  invoke<void>("reorder_tasks", { status, orderedIds });
export const readTaskPlan = (id: string) => invoke<string>("read_task_plan", { id });
export const listTaskRelations = (taskId: string) => invoke<TaskRelation[]>("list_task_relations", { taskId });
export const addTaskRelation = (taskId: string, otherId: string, kind: RelationKind) =>
  invoke<void>("add_task_relation", { taskId, otherId, kind });
export const removeTaskRelation = (taskId: string, otherId: string, kind: RelationKind) =>
  invoke<void>("remove_task_relation", { taskId, otherId, kind });
/**
 * Borra el worktree y la rama de la tarea ("Clean up"). Sin `force` rechaza si hay commits
 * sin publicar (`unpushed`) o cambios sin commitear (`dirty`).
 */
export const cleanupWorktree = (taskId: string, force = false) =>
  invoke<Task>("cleanup_worktree", { taskId, force });
/** Ruta absoluta. No lanza nada ni escribe el config de Claude Code. */
export const repoTrust = (path: string) => invoke<RepoTrust>("repo_trust", { path });
/** `git version 2.x.y`; rechaza si no hay git en el PATH. */
export const gitVersion = () => invoke<string>("git_version");
/** Actividad de Claude Code por repo del proyecto (un solo `claude agents`). */
export const projectActivity = (projectId: string) => invoke<ProjectActivity>("project_activity", { projectId });
/** Todo en cero/false/null si la tarea no tiene worktree. */
export const worktreeStatus = (taskId: string) => invoke<WorktreeStatus>("worktree_status", { taskId });

// ---------- Ejecutores y runs ----------

/** Agentes, workflows y Claude; con `repoId` suma los del repo. */
export const listExecutors = (repoId: string | null) => invoke<ExecutorInfo[]>("list_executors", { repoId });
/** Runs de la tarea (o todos con `null`), más recientes primero. */
export const listTaskRuns = (taskId: string | null, projectId: string | null = null) =>
  invoke<Run[]>("list_task_runs", { taskId, projectId });
/** Como `listTaskRuns` (hasta 500), sin `prompt` ni `extraInstructions`. Un run sin tarea cuenta en el proyecto de su repo. */
export const listRunsLight = (projectId: string | null = null, taskId: string | null = null) =>
  invoke<RunLight[]>("list_runs_light", { projectId, taskId });
/** Run completo (con `prompt`). */
export const getRun = (runId: string) => invoke<Run>("get_run", { runId });
/** El último run de cada tarea, sin límite de historial. */
export const latestRunsByTask = (projectId: string | null = null) =>
  invoke<RunLight[]>("latest_runs_by_task", { projectId });
/** `null`: global (incluye sesiones de Claude Code ajenas a la app); con proyecto, solo sus runs y tareas. */
export const workSummary = (projectId: string | null = null) => invoke<WorkSummary>("work_summary", { projectId });
/** Cola global: runs `queued`, en orden de salida. */
export const listQueue = () => invoke<Run[]>("list_queue");
/** Encola un run de trabajo (o lo lanza si hay slot). */
export const launchTask = (taskId: string, input: LaunchInput = {}) => invoke<Run>("launch_task", { taskId, input });
/** Siguiente paso de la cadena con otro ejecutor, sobre la misma rama/worktree. */
export const handOff = (taskId: string, executor: Executor, extraInstructions: string | null = null) =>
  invoke<Run>("hand_off", { taskId, executor, extraInstructions });
/** Lanza el revisor ahora. `reviewer` null → el configurado. */
export const reviewNow = (taskId: string, reviewer: string | null = null) =>
  invoke<Run>("review_now", { taskId, reviewer });
/** Saca de la cola un run `queued`, o detiene uno lanzado (guarda el patch a medias). */
export const cancelRun = (runId: string) => invoke<Run>("cancel_run", { runId });
/**
 * Un run migrado que quedó en cola (`status: "queued"` con `legacyLabel`) no se lanza solo:
 * esto lo re-encola como run nuevo (el viejo queda cancelado). Para descartarlo, `cancelRun`.
 */
export const confirmRun = (runId: string) => invoke<Run>("confirm_run", { runId });
/** Nuevo orden de la cola: ids de todos los runs `queued`. */
export const reorderQueue = (runIds: string[]) => invoke<void>("reorder_queue", { runIds });
export const runDiff = (runId: string) => invoke<RunDiff>("run_diff", { runId });
/**
 * Transcript de un run de agente, Claude o revisor (sesión principal; `agentId` = id del run).
 * Rechaza para workflows (usar `getAgentTranscript`). `null` si la sesión aún no tiene archivo.
 * `limit`: items más recientes (default 200, máx. 2000).
 */
export const getRunTranscript = (runId: string, limit?: number) =>
  invoke<Transcript | null>("get_run_transcript", { runId, limit: limit ?? null });
/** Abre la carpeta del run (o `file` dentro de ella) en el editor de Settings. */
export const openInEditor = (runId: string, file: string | null = null) =>
  invoke<void>("open_in_editor", { runId, file });
/** Abre la carpeta del run en Finder. */
export const openWorktree = (runId: string) => invoke<void>("open_worktree", { runId });

// ---------- Proveedores ----------

export const providerStatus = (provider: string) => invoke<ProviderStatus>("provider_status", { provider });
/** Valida la key y la guarda en el keychain; `null` la borra. */
export const providerSetKey = (provider: string, key: string | null) =>
  invoke<ProviderStatus>("provider_set_key", { provider, key });
export const providerClearKey = (provider: string) => invoke<ProviderStatus>("provider_clear_key", { provider });
export const providerScopes = (provider: string) => invoke<ScopeRef[]>("provider_scopes", { provider });
export const listSourceLinks = (projectId: string | null) => invoke<SourceLink[]>("list_source_links", { projectId });
/** Arma la propuesta de mapeo de estados; queda "pendiente" hasta `saveStateMap`. */
export const createSourceLink = (input: NewSourceLink) => invoke<SourceLink>("create_source_link", { input });
export const updateSourceLink = (id: string, patch: SourceLinkPatch) =>
  invoke<SourceLink>("update_source_link", { id, patch });
/** Disconnect: desvincula sus tareas (quedan locales) y borra el link, en una transacción. */
export const deleteSourceLink = (id: string) => invoke<void>("delete_source_link", { id });
/** Unlink: la tarea pasa a ser local. */
export const unlinkTask = (taskId: string) => invoke<Task>("unlink_task", { taskId });
/** Hasta 100 ítems del scope. `stateKinds` null o vacío = abiertos (triage, backlog, unstarted, started). */
export const providerListImportable = (linkId: string, query: string | null = null, stateKinds: ExtKind[] | null = null) =>
  invoke<ImportableItem[]>("provider_list_importable", { linkId, query, stateKinds });
export const importTasks = (projectId: string, linkId: string, items: { externalId: string; repoId: string }[]) =>
  invoke<ImportResult>("import_tasks", { projectId, linkId, items });
/** `null`: todas las fuentes. */
export const syncNow = (linkId: string | null = null) => invoke<SyncReport>("sync_now", { linkId });
export const sourceStates = (linkId: string) => invoke<SourceStatesReport>("source_states", { linkId });
/** Guarda y confirma el mapeo (fija `confirmedAt` y `knownStates`). */
export const saveStateMap = (linkId: string, map: StateMap) => invoke<SourceLink>("save_state_map", { linkId, map });

// ---------- Migración ----------

/** Copia `folder` a `legacy-backup-<ts>/` y la importa en una transacción. Idempotente. */
export const importLegacyData = (folder: string) => invoke<LegacyImportReport>("import_legacy_data", { folder });

// ---------- Settings ----------

export const getSettings = () => invoke<Settings>("get_settings");
export const setSettings = (settings: Settings) => invoke<Settings>("set_settings", { settings });

// ---------- Eventos ----------

export type ChangedKind = "tasks" | "runs" | "queue" | "sources" | "projects";

/** Payload de `nodal://changed`. Sin `projectId`: puede afectar a cualquier proyecto. */
export interface ChangedEvent {
  kind: ChangedKind;
  projectId?: string;
}

/**
 * Avisa cuando algo cambió en el backend (cola, sync o comandos que mutan), con debounce
 * del lado Rust. No trae datos: volver a pedir lo que se muestra. Devuelve el unlisten.
 */
export const onChanged = (cb: (e: ChangedEvent) => void): Promise<UnlistenFn> =>
  listen<ChangedEvent>("nodal://changed", (ev) => cb(ev.payload));
