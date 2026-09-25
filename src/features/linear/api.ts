import { invoke } from "@tauri-apps/api/core";

// Mirror of src-tauri/src/linear/model.rs.

export type StateType = "triage" | "backlog" | "unstarted" | "started" | "completed" | "canceled";

// Mirror of src-tauri/src/linear/detail.rs.
export interface StateRef { name: string; type: StateType | string; color: string }
export interface IssueRef { id: string; identifier: string; title: string; state: StateRef }
export interface UserRef { id: string; name: string; displayName: string }
export type RelationKind = "blocks" | "blocked_by" | "related" | "duplicate";
export interface IssueDetail {
  id: string;
  identifier: string;
  /** Raw Markdown from Linear. */
  description: string | null;
  parent: IssueRef | null;
  children: (IssueRef & { assignee: UserRef | null })[];
  childrenTruncated: boolean;
  /** `inverse` only matters for `duplicate`: true = the other issue duplicates this one. */
  relations: { kind: RelationKind; inverse: boolean; issue: IssueRef }[];
  labels: { id: string; name: string; color: string }[];
  estimate: number | null;
  /** "YYYY-MM-DD" */
  dueDate: string | null;
  createdAt: string;
  creator: UserRef | null;
  /** Up to 20 (Linear's default page size), in ascending chronological order. */
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

/** Linear commands reject with `{kind, message}`; anything else gets normalized. */
export function toLinearError(err: unknown): LinearError {
  if (err && typeof err === "object" && "kind" in err && "message" in err) {
    return err as LinearError;
  }
  return { kind: "unknown", message: String(err) };
}

export const linearApi = {
  /** Accepts a UUID or an identifier ("ACME-8"). */
  issueDetail: (issueId: string) => invoke<IssueDetail>("linear_issue_detail", { issueId }),
};
