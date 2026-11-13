import type { FileDiff } from "../../domain/api";

// Mirror of the transcript types in src-tauri (serde rename_all = camelCase): what Claude Code
// reports about a chat session.

export interface ToolResultInfo {
  text: string;
  isError: boolean;
  truncated: boolean;
  /** The file change of an `Edit`/`Write`/`MultiEdit`, when Claude Code reported it. */
  patch: ToolPatch | null;
}

export interface ToolPatch {
  file: FileDiff;
  /** Lines past the cap were dropped (the counts still cover the whole change). */
  truncated: boolean;
}

export type TranscriptItem =
  | { kind: "text"; text: string; truncated: boolean }
  | { kind: "thinking"; text: string; truncated: boolean }
  | { kind: "user"; text: string; truncated: boolean }
  | {
      kind: "toolUse";
      id: string | null;
      name: string;
      summary: string | null;
      input: string | null;
      result: ToolResultInfo | null;
    };

export interface Transcript {
  agentId: string;
  label: string | null;
  model: string | null;
  phase: string | null;
  prompt: string | null;
  items: TranscriptItem[];
  totalItems: number;
  /** Older items that weren't returned. */
  omitted: number;
  /** Tool calls in what was read, including the omitted items. */
  toolCalls: number;
  finalOutput: string | null;
  /** Very large file: only the head and tail were read. */
  partial: boolean;
  bytes: number;
}
