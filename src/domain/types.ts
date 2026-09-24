// Espejo exacto de `src-tauri/src/domain/mod.rs`. Cualquier cambio va en los dos lados.
// Convenciones: campos camelCase, enums de valor snake_case, enums con datos
// discriminados por `kind`, fechas en epoch ms. `Option<T>` de Rust llega como `T | null`
// (salvo `LaunchOptions`, cuyos campos se omiten si están vacíos).

// ---------- Proyectos y repos ----------

export interface Project {
  id: string;
  name: string;
  /** Prefijo de los ids de tarea (`PAY` → `PAY-1`). */
  key: string;
  nextTaskNumber: number;
  color: string;
  description: string | null;
  /** `null` → `Settings.defaultExecutor` → Claude. */
  defaultExecutor: Executor | null;
  /** `null` → `code-reviewer`. */
  reviewer: string | null;
  createdAt: number;
  archivedAt: number | null;
}

/** Flags opcionales de `claude --bg`. */
export interface LaunchOptions {
  model?: string;
  effort?: string;
  permissionMode?: string;
}

/** `model`, `effort` y `permissionMode` van planos (flatten en Rust). */
export interface Repo extends LaunchOptions {
  id: string;
  projectId: string;
  /** Raíz git canónica. */
  path: string;
  name: string;
  defaultExecutor: Executor | null;
  defaultIsolation: Isolation;
  defaultFinish: Finish;
  defaultReview: boolean;
  reviewer: string | null;
  position: number;
  createdAt: number;
}

// ---------- Ejecutores y opciones ----------

export type AgentSource = "user" | "repo" | "plugin";

export type Executor =
  | { kind: "agent"; name: string; source: AgentSource }
  | { kind: "workflow"; name: string }
  | { kind: "claude" };

export type Isolation = "worktree" | "in_place";

/** `changes`: sin commitear · `commit`: commit sin push · `pr`: commit, push y PR. */
export type Finish = "changes" | "commit" | "pr";

// ---------- Tareas ----------

export type TaskStatus = "backlog" | "todo" | "in_progress" | "in_review" | "blocked" | "done" | "canceled";

export const TASK_STATUSES: readonly TaskStatus[] = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "blocked",
  "done",
  "canceled",
];

export type Priority = "urgent" | "high" | "medium" | "low" | "none";

export type PlanRef = { kind: "text" } | { kind: "file"; path: string };

export interface WorktreeRef {
  path: string;
  branch: string;
  base: string;
}

export type ExtKind = "triage" | "backlog" | "unstarted" | "started" | "completed" | "canceled" | "unknown";

export interface ExternalState {
  id: string;
  name: string;
  kind: ExtKind;
  color: string | null;
}

export interface TaskSource {
  provider: string;
  /** Para borrar el link, antes se desvinculan sus tareas. */
  linkId: string | null;
  externalId: string;
  /** Id legible del proveedor (`ENG-142`). */
  identifier: string;
  url: string;
  externalState: ExternalState | null;
  lastSyncedAt: number | null;
  syncError: string | null;
  /** El estado externo actual no está en el mapeo pull ("estado externo sin mapear"). */
  unmapped: boolean;
  /** Proyecto del proveedor (proyecto de Linear) visto en el último import/pull. */
  project: ExtProject | null;
  /** Regla de proyecto por la que llegó a su repo. */
  ruleId: string | null;
  /** La issue cambió de proyecto en el proveedor: falta decidir (`resolveMovedTask`). */
  moved: MovedInfo | null;
}

export interface ExtProject {
  id: string;
  name: string;
}

export interface MovedInfo {
  fromProject: ExtProject;
  /** `null`: quedó sin proyecto. */
  toProject: ExtProject | null;
  /** Repo que le tocaría por las reglas; `null` si ninguna aplica. */
  suggestedRepoId: string | null;
}

export interface Task {
  id: string;
  projectId: string;
  repoId: string;
  /** El id visible es `taskKey(project.key, number)`. */
  number: number;
  title: string;
  status: TaskStatus;
  priority: Priority;
  labels: string[];
  position: number;
  plan: PlanRef;
  planOverridden: boolean;
  acceptance: string[];
  /** `null` → default del repo → del proyecto → Claude. */
  assignee: Executor | null;
  isolation: Isolation | null;
  finish: Finish | null;
  review: boolean | null;
  worktree: WorktreeRef | null;
  source: TaskSource | null;
  createdAt: number;
  updatedAt: number;
  closedAt: number | null;
}

export const taskKey = (projectKey: string, number: number) => `${projectKey}-${number}`;

/** `blocks`: `taskId` bloquea a `otherId`. */
export type RelationKind = "related" | "blocks";

export interface TaskRelation {
  taskId: string;
  otherId: string;
  kind: RelationKind;
}

// ---------- Runs ----------

export type RunKind = "work" | "review";

export type RunStatus = "queued" | "launching" | "launched" | "finished" | "failed" | "canceled";

export type RunOutcome = "green" | "yellow" | "red" | "stopped" | "unknown";

export interface Verdict {
  pass: boolean;
  unmet: string[];
  nits: string[];
  summary: string | null;
}

export interface Run {
  id: string;
  taskId: string | null;
  repoId: string | null;
  cwd: string;
  executor: Executor;
  kind: RunKind;
  parentRunId: string | null;
  prompt: string;
  extraInstructions: string | null;
  options: LaunchOptions;
  finish: Finish;
  /** Resuelta al encolar; `null` en workflows. */
  isolation: Isolation | null;
  /** Resuelto al encolar: al terminar, se encola el revisor. */
  review: boolean;
  verdict: Verdict | null;
  status: RunStatus;
  queuePosition: number;
  claudeRunId: string | null;
  sessionId: string | null;
  queuedAt: number;
  launchedAt: number | null;
  finishedAt: number | null;
  outcome: RunOutcome | null;
  summary: string | null;
  prUrl: string | null;
  branch: string | null;
  error: string | null;
  legacyLabel: string | null;
  /** Tokens del transcript (agente/Claude/revisor), sumados al cerrar. */
  tokens: number | null;
}

/** `Run` sin `prompt` ni `extraInstructions`, para listas. */
export type RunLight = Omit<Run, "prompt" | "extraInstructions">;

// ---------- Fuentes externas ----------

export interface ScopeRef {
  kind: string;
  id: string;
  name: string;
}

export type RuleKind = "label" | "project";

/**
 * Ruteo al importar. Precedencia: regla de proyecto > regla de label > `defaultRepoId`.
 * Al guardar, una regla sin `id` (o con otro `kind`/`value`) es nueva: el backend le da `id`
 * y `createdAt` (desde ahí auto-importa; lo anterior lo trae `importRule`).
 */
export interface RepoRule {
  /** `""` en una regla nueva. */
  id: string;
  kind: RuleKind;
  /** Label, o id del proyecto del proveedor. */
  value: string;
  /** Nombre visible (el del proyecto; en las de label, el label). */
  name: string;
  repoId: string;
  /** Lo fija el backend. */
  createdAt: number;
}

export interface StateMap {
  /** `externalState.id` → estado Nodal. Ausente = sin mapear. */
  pull: Record<string, TaskStatus>;
  /** Estado Nodal → `externalState.id`; `null` = "No sincronizar". */
  push: Partial<Record<TaskStatus, string | null>>;
  /** `null` = mapeo pendiente (no se hace push). */
  confirmedAt: number | null;
  knownStates: ExternalState[];
}

/** Estados del proveedor que cambiaron contra `knownStates`; se limpia con `saveStateMap`. */
export interface StateChanges {
  added: ExternalState[];
  removed: ExternalState[];
}

export interface SourceLink {
  id: string;
  projectId: string;
  provider: string;
  scope: ScopeRef;
  defaultRepoId: string | null;
  repoRules: RepoRule[];
  stateMap: StateMap;
  autoImport: boolean;
  createdAt: number;
  lastSyncedAt: number | null;
  lastSyncError: string | null;
  /** `null` = nada pendiente. */
  pendingStateChanges: StateChanges | null;
}

export type OutboxPayload = { kind: "set_state"; stateId: string } | { kind: "comment"; body: string };

export interface OutboxItem {
  id: number;
  taskId: string;
  provider: string;
  payload: OutboxPayload;
  attempts: number;
  nextAttemptAt: number;
  lastError: string | null;
  createdAt: number;
}

// ---------- Settings ----------

export interface Settings {
  concurrency: number;
  /** `code`, `cursor`... `null` → el del sistema. */
  editor: string | null;
  reviewer: string;
  /** Último fallback del ejecutor: repo → proyecto → este → Claude. */
  defaultExecutor: Executor | null;
}
