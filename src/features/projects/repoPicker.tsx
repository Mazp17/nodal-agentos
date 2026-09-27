// Picking a repo folder: native macOS picker, then `resolve_git_root` and the
// design's messages ("not inside a git repository", "Use repo root", "already added to…").

import { useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { scanGitRepos } from "../../domain/api";
import { basename, type ProjectsState } from "../../domain/hooks/projects";
import "./projects.css";

/** Canonical git root of the folder, or `null` if it isn't in a repo. */
export const resolveGitRoot = (path: string) => invoke<string | null>("resolve_git_root", { path });

/** Name of the project that already has this repo, or `null` if no project has it. */
export function projectOfRepo(ctx: Pick<ProjectsState, "repos" | "projectById">, root: string): string | null {
  const r = ctx.repos.find((x) => x.path === root);
  return r ? (ctx.projectById.get(r.projectId)?.name ?? "another project") : null;
}

/** Native folder picker; `null` if cancelled. */
export async function pickFolder(title: string): Promise<string | null> {
  const picked = await open({ directory: true, multiple: false, title });
  return typeof picked === "string" ? picked : null;
}

/**
 * The root comes back canonical (symlinks resolved) and the picked folder doesn't: with the same
 * final name it counts as the root (e.g. `/tmp/x` vs `/private/tmp/x`).
 */
const isSubfolder = (path: string, root: string) =>
  path.replace(/\/+$/, "") !== root.replace(/\/+$/, "") && basename(path) !== basename(root);

export type RepoNotice =
  | { kind: "bad"; text: string }
  | { kind: "sub"; text: string; root: string }
  | { kind: "dup"; text: string };

interface Options {
  /** If the repo is already in use, where (`"Payments"`), or `""` if it's an unnamed draft. */
  whereAdded: (root: string) => string | null;
  onPicked: (root: string) => void | Promise<void>;
}

/** Repo picker with its notice. `pick()` opens the native dialog. */
export function useRepoPicker({ whereAdded, onPicked }: Options) {
  const [notice, setNotice] = useState<RepoNotice | null>(null);
  const [busy, setBusy] = useState(false);

  const accept = async (root: string) => {
    const where = whereAdded(root);
    if (where !== null) {
      setNotice({ kind: "dup", text: `${root} is already added${where ? ` to ${where}` : ""}.` });
      return;
    }
    setNotice(null);
    await onPicked(root);
  };

  const pick = async () => {
    setNotice(null);
    let path: string | null;
    try {
      path = await pickFolder("Choose a repository folder");
    } catch (e) {
      setNotice({ kind: "bad", text: `Couldn't open the folder picker: ${String(e)}` });
      return;
    }
    if (!path) return;
    setBusy(true);
    try {
      const root = await resolveGitRoot(path);
      if (!root) {
        setNotice({ kind: "bad", text: `${path} is not inside a git repository. Pick the root of a git checkout.` });
      } else if (isSubfolder(path, root)) {
        setNotice({
          kind: "sub",
          root,
          text: `${basename(path)} is a subfolder of the git repo ${root}. Nodal works from the repo root.`,
        });
      } else {
        await accept(root);
      }
    } catch (e) {
      setNotice({ kind: "bad", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const useRoot = async () => {
    if (notice?.kind !== "sub") return;
    setBusy(true);
    try {
      await accept(notice.root);
    } finally {
      setBusy(false);
    }
  };

  return { pick, notice, busy, useRoot, dismiss: () => setNotice(null), setNotice };
}

export function RepoNoticeBar({
  notice,
  onUseRoot,
  onDismiss,
}: {
  notice: RepoNotice;
  onUseRoot: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className={`repo-notice repo-notice-${notice.kind}`} role={notice.kind === "sub" ? "status" : "alert"}>
      <span className="repo-notice-text">{notice.text}</span>
      {notice.kind === "sub" && (
        <button type="button" className="btn btn-primary btn-sm" onClick={onUseRoot}>
          Use repo root
        </button>
      )}
      <button type="button" className="icon-btn" aria-label="Dismiss" onClick={onDismiss}>
        ✕
      </button>
    </div>
  );
}

/** Row for a draft repo (onboarding, New project). */
export function DraftRepoRow({ path, onRemove }: { path: string; onRemove: () => void }) {
  return (
    <div className="draft-repo">
      <span className="draft-repo-name">{basename(path)}</span>
      <span className="draft-repo-path mono ellipsis" title={path}>
        {path}
      </span>
      <button type="button" className="btn btn-ghost btn-xs draft-repo-remove" onClick={onRemove}>
        Remove
      </button>
    </div>
  );
}

/**
 * Repos found under a project folder, to pick which ones to add. Repos that `whereAdded` places in
 * another project are listed but can't be checked: a repo belongs to one project only.
 */
export function useRepoScan(whereAdded: (root: string) => string | null) {
  const [found, setFound] = useState<string[] | null>(null);
  const [checked, setChecked] = useState<Set<string>>(new Set());
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // A slower earlier scan must not overwrite the list of a folder picked after it.
  const seq = useRef(0);

  /** `skip`: repos not to offer at all (e.g. already in this project). */
  const scan = async (root: string, skip: readonly string[] = []) => {
    const n = ++seq.current;
    setScanning(true);
    setError(null);
    try {
      const repos = (await scanGitRepos(root)).filter((p) => !skip.includes(p));
      if (n !== seq.current) return;
      setFound(repos);
      setChecked(new Set(repos.filter((p) => whereAdded(p) === null)));
    } catch (e) {
      if (n !== seq.current) return;
      setFound(null);
      setError(String(e));
    } finally {
      if (n === seq.current) setScanning(false);
    }
  };

  const clear = () => {
    seq.current++;
    setFound(null);
    setChecked(new Set());
    setError(null);
    setScanning(false);
  };

  const toggle = (path: string) =>
    setChecked((cs) => {
      const next = new Set(cs);
      if (!next.delete(path)) next.add(path);
      return next;
    });

  const selected = (found ?? []).filter((p) => checked.has(p) && whereAdded(p) === null);

  return { found, checked, scanning, error, scan, clear, toggle, selected };
}

export type RepoScan = ReturnType<typeof useRepoScan>;

/** Checklist of the repos a scan found. `empty`: text when the folder has none to offer. */
export function ScannedRepoList({
  scan,
  whereAdded,
  empty,
}: {
  scan: RepoScan;
  whereAdded: (root: string) => string | null;
  empty: string;
}) {
  if (scan.scanning) return <span className="field-hint">Looking for git repos…</span>;
  if (scan.error)
    return (
      <div className="repo-notice repo-notice-bad" role="alert">
        <span className="repo-notice-text">{scan.error}</span>
      </div>
    );
  if (!scan.found) return null;
  if (scan.found.length === 0) return <span className="field-hint">{empty}</span>;
  return (
    <div className="scan-repos" role="group" aria-label="Repos found in the folder">
      {scan.found.map((path) => {
        const where = whereAdded(path);
        return (
          <label key={path} className={`draft-repo scan-repo ${where !== null ? "scan-repo-taken" : ""}`}>
            <input
              type="checkbox"
              checked={where === null && scan.checked.has(path)}
              disabled={where !== null}
              onChange={() => scan.toggle(path)}
            />
            <span className="draft-repo-name">{basename(path)}</span>
            <span className="draft-repo-path mono ellipsis" title={path}>
              {path}
            </span>
            {where !== null && <span className="scan-repo-where">{where ? `In ${where}` : "Already added"}</span>}
          </label>
        );
      })}
    </div>
  );
}
