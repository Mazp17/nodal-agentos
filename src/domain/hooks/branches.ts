// Repo checkout resources for the chat's files panel and file tabs: the branch a checkout is
// on, its worktrees, directory listings, file text and uncommitted changes. `worktree` (an
// absolute path from `listRepoWorktrees`) picks one of the repo's worktrees; `null` is the
// repo's own checkout. Cache keys carry it, so both can be open at once.

import {
  chatWorkspace,
  listRepoDir,
  listRepoWorktrees,
  readRepoFile,
  repoBranch,
  repoChanges,
  repoCommitDiff,
  repoCommits,
  type ChatWorkspace,
  type CommitInfo,
  type DirEntry,
  type FileDiff,
  type RepoFile,
  type RepoWorktree,
} from "../api";
import { POLL, usePolled, type Loadable } from "./store";

/** Cache-key form of a checkout: the repo id, plus the worktree path if any. */
export const rootKey = (repoId: string, worktree: string | null) => (worktree ? `${repoId}@${worktree}` : repoId);

/** Current branch of a repo's checkout; `null` while unknown or detached. */
export const useRepoBranch = (repoId: string | null): Loadable<string | null> =>
  usePolled<string | null>(repoId ? `repo-branch:${repoId}` : null, () => repoBranch(repoId!), ["repos"], POLL.slow);

/** The repo's checkout and its git worktrees (main first). */
export const useRepoWorktrees = (repoId: string | null): Loadable<RepoWorktree[]> =>
  usePolled<RepoWorktree[]>(repoId ? `repo-worktrees:${repoId}` : null, () => listRepoWorktrees(repoId as string), ["repos", "chats"], POLL.slow);

/** Children of `dir` in a checkout; re-read on `repos` notices only. */
export const useRepoDir = (repoId: string, dir: string, enabled: boolean, worktree: string | null = null): Loadable<DirEntry[]> =>
  usePolled<DirEntry[]>(
    enabled ? `repo-dir:${rootKey(repoId, worktree)}:${dir}` : null,
    () => listRepoDir(repoId, dir, worktree),
    ["repos"],
    0,
  );

/** Where a chat works now (it may have moved into a worktree) and its uncommitted changes there.
 * Re-read on `chats` notices (a chat view sends one when an answer ends). */
export const useChatWorkspace = (chatId: string | null): Loadable<ChatWorkspace> =>
  usePolled<ChatWorkspace>(chatId ? `chat-ws:${chatId}` : null, () => chatWorkspace(chatId!), ["chats"], POLL.slow);

/** Cache key of a file's text (the editor writes the saved file there). */
export const repoFileKey = (repoId: string, path: string, worktree: string | null) => `repo-file:${rootKey(repoId, worktree)}:${path}`;

/** A file's text for the editor; read once per open tab (re-read on `repos` notices). */
export const useRepoFile = (repoId: string, path: string, worktree: string | null = null): Loadable<RepoFile> =>
  usePolled<RepoFile>(repoFileKey(repoId, path, worktree), () => readRepoFile(repoId, path, worktree), ["repos"], 0);

/** Uncommitted changes of a checkout; polled, since agents and editors change them outside Nodal. */
export const useRepoChanges = (repoId: string, enabled: boolean, worktree: string | null = null): Loadable<FileDiff[]> =>
  usePolled<FileDiff[]>(
    enabled ? `repo-changes:${rootKey(repoId, worktree)}` : null,
    () => repoChanges(repoId, worktree),
    ["repos", "chats"],
    POLL.live,
  );

/** The checkout's commits (see `repoCommits`); re-read with the changes, since commits land with them. */
export const useRepoCommits = (repoId: string, enabled: boolean, worktree: string | null = null): Loadable<CommitInfo[]> =>
  usePolled<CommitInfo[]>(enabled ? `repo-commits:${rootKey(repoId, worktree)}` : null, () => repoCommits(repoId, worktree), ["repos", "chats"], POLL.slow);

/** A commit never changes: read once. */
export const useCommitDiff = (repoId: string, sha: string, worktree: string | null = null): Loadable<FileDiff[]> =>
  usePolled<FileDiff[]>(`commit-diff:${rootKey(repoId, worktree)}:${sha}`, () => repoCommitDiff(repoId, sha, worktree), [], 0);
