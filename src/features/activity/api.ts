import { invoke } from "@tauri-apps/api/core";
import type { RepoActivity } from "./types";

/** Sesiones y subagentes de Claude Code trabajando en el repo (o sus worktrees). */
export const repoActivity = (repoPath: string) => invoke<RepoActivity>("repo_activity", { repoPath });
