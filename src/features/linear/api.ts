import { invoke } from "@tauri-apps/api/core";

// Espejo de src-tauri/src/linear/model.rs.

export type StateType = "triage" | "backlog" | "unstarted" | "started" | "completed" | "canceled";

// Espejo de src-tauri/src/linear/detail.rs.
export interface StateRef { name: string; type: StateType | string; color: string }
export interface IssueRef { id: string; identifier: string; title: string; state: StateRef }
export interface UserRef { id: string; name: string; displayName: string }
export type RelationKind = "blocks" | "blocked_by" | "related" | "duplicate";
export interface IssueDetail {
  id: string;
  identifier: string;
  /** Markdown crudo de Linear. */
  description: string | null;
  parent: IssueRef | null;
  children: (IssueRef & { assignee: UserRef | null })[];
  childrenTruncated: boolean;
  /** `inverse` sólo importa para `duplicate`: true = la otra issue duplica a ésta. */
  relations: { kind: RelationKind; inverse: boolean; issue: IssueRef }[];
  labels: { id: string; name: string; color: string }[];
  estimate: number | null;
  /** "YYYY-MM-DD" */
  dueDate: string | null;
  createdAt: string;
  creator: UserRef | null;
  /** Hasta 20 (página por defecto de Linear), en orden cronológico ascendente. */
  comments: { id: string; body: string; author: string | null; createdAt: string }[];
  commentsTruncated: boolean;
}

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
  /** Acepta UUID o identifier ("ACME-8"). */
  issueDetail: (issueId: string) => invoke<IssueDetail>("linear_issue_detail", { issueId }),
};
