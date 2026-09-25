import { invoke } from "@tauri-apps/api/core";
import type { RepoActivity } from "./types";

/** Claude Code sessions and subagents working in the repo (or its worktrees). */
export const repoActivity = (repoPath: string) => invoke<RepoActivity>("repo_activity", { repoPath });
