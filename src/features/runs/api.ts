import { invoke } from "@tauri-apps/api/core";
import type { RunDetail, RunRef, RunSummary } from "./types";

export const launchRun = (cwd: string, prompt: string) => invoke<RunRef>("launch_run", { cwd, prompt });

export const listRuns = () => invoke<RunSummary[]>("list_runs");

/** `null` si la sesión no tiene carpeta todavía o no lanzó ningún workflow. */
export const getRunDetail = (sessionId: string, cwd: string) =>
  invoke<RunDetail | null>("get_run_detail", { sessionId, cwd });
