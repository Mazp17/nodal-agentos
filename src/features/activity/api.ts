import { invoke } from "@tauri-apps/api/core";
import type { ActivitySummary, RepoActivity } from "./types";

/** Sesiones y subagentes de Claude Code trabajando en el repo (o sus worktrees). */
export const repoActivity = (repoPath: string) => invoke<RepoActivity>("repo_activity", { repoPath });

/** Sesiones trabajando y subagentes activos en todos esos repos (una sola pasada). */
export const activitySummary = (repoPaths: string[]) => invoke<ActivitySummary>("activity_summary", { repoPaths });
