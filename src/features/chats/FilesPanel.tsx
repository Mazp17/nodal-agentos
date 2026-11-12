import { useEffect, useRef, useState } from "react";
import { ContextMenu, DropdownMenu } from "radix-ui";
import { rootKey, useRepoChanges, useRepoDir, useRepoWorktrees } from "../../domain/hooks/branches";
import type { FileDiff } from "../../domain/api";
import { readJsonPref, writePref } from "../../shell/storage";

const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { revertChanges } from "./FileView";
import type { RepoWorktree } from "../../domain/api";
import type { DiffFileStatus } from "../../domain/api";
import type { Repo } from "../../domain/types";
import "../runs/diff.css";

interface Props {
  repo: Repo;
  /** The file open in the active tab (relative path), highlighted in the tree. */
  activePath: string | null;
  onOpenFile: (path: string) => void;
  /** Opens a changed file's diff (from the "Changes" view). */
  onOpenDiff: (path: string) => void;
  /** All files, or only the uncommitted changes (the chat's "N files changed" opens this). */
  mode: FilesMode;
  onMode: (mode: FilesMode) => void;
  /** The checkout shown: one of the repo's worktrees (absolute path), `null` for the repo's own. */
  worktree: string | null;
  onWorktree: (worktree: string | null) => void;
  /** A folder the panel should show if a worktree contains it (the chat's working folder). */
  prefer: string | null;
  onClose: () => void;
}

export type FilesMode = "tree" | "changes";

const STATUS_BADGE: Record<DiffFileStatus, { letter: string; cls: string; label: string }> = {
  added: { letter: "A", cls: "df-added", label: "Added" },
  modified: { letter: "M", cls: "df-modified", label: "Modified" },
  deleted: { letter: "D", cls: "df-deleted", label: "Deleted" },
  renamed: { letter: "R", cls: "df-renamed", label: "Renamed" },
};

/** The repo's files next to the chat: a lazy tree (one listing per expanded folder), or only
 * the uncommitted changes. */
export function FilesPanel({ repo, activePath, onOpenFile, onOpenDiff, mode, onMode: setMode, worktree, onWorktree, prefer, onClose }: Props) {
  const worktrees = useRepoWorktrees(repo.id).data;
  // Follow the chat into its worktree: the one whose folder holds `prefer` (longest match).
  // Only when `prefer` changes, so a checkout picked by hand stays picked.
  const applied = useRef<string | null>(null);
  useEffect(() => {
    if (!prefer || !worktrees || applied.current === prefer) return;
    applied.current = prefer;
    const hit = worktrees
      .filter((w) => prefer === w.path || prefer.startsWith(`${w.path}/`))
      .sort((a, b) => b.path.length - a.path.length)[0];
    if (hit) onWorktree(hit.isMain ? null : hit.path);
  }, [prefer, worktrees, onWorktree]);
  // A worktree that went away (removed outside Nodal): back to the repo's checkout.
  useEffect(() => {
    if (worktree && worktrees && !worktrees.some((w) => w.path === worktree)) onWorktree(null);
  }, [worktree, worktrees, onWorktree]);
  // Expanded folders, remembered per checkout (the panel remounts per repo).
  const openKey = `filesOpenDirs:${rootKey(repo.id, worktree)}`;
  const [open, setOpen] = useState<ReadonlySet<string>>(() => new Set(readJsonPref<string[]>(openKey, [], isStrings)));
  const [openFor, setOpenFor] = useState(openKey);
  if (openFor !== openKey) {
    // Another checkout picked: load its folders (during render, so the tree never flashes the old set).
    setOpenFor(openKey);
    setOpen(new Set(readJsonPref<string[]>(openKey, [], isStrings)));
  }
  useEffect(() => writePref(openFor, JSON.stringify([...open])), [open, openFor]);
  const toggle = (path: string) =>
    setOpen((s) => {
      const n = new Set(s);
      if (n.has(path)) n.delete(path);
      else n.add(path);
      return n;
    });
  return (
    <aside className="files-panel" aria-label={`${repo.name} files`}>
      <div className="files-head">
        <span className="files-title ellipsis" title={repo.path}>
          {repo.name}
        </span>
        <div className="files-modes" role="group" aria-label="Show">
          <button type="button" className={`icon-btn ${mode === "tree" ? "on" : ""}`} aria-pressed={mode === "tree"} aria-label="All files" title="All files" onClick={() => setMode("tree")}>
            <FileIcon dir />
          </button>
          <button type="button" className={`icon-btn ${mode === "changes" ? "on" : ""}`} aria-pressed={mode === "changes"} aria-label="Changes only" title="Changes only (git)" onClick={() => setMode("changes")}>
            <svg width="13" height="13" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" aria-hidden focusable="false">
              <circle cx="4" cy="3" r="1.5" />
              <circle cx="4" cy="11" r="1.5" />
              <circle cx="10" cy="5" r="1.5" />
              <path d="M4 4.5v5M10 6.5c0 2-2 2.5-6 3" />
            </svg>
          </button>
        </div>
        <button type="button" className="icon-btn" aria-label="Close files" title="Close" onClick={onClose}>
          <svg width="9" height="9" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" aria-hidden focusable="false">
            <path d="M2 2l6 6M8 2l-6 6" />
          </svg>
        </button>
      </div>
      {worktrees && worktrees.length > 1 && <RootPicker repo={repo} worktrees={worktrees} value={worktree} onChange={onWorktree} />}
      {mode === "tree" ? (
        <ul className="files-tree" role="tree" aria-label="Files">
          <Dir key={worktree ?? ""} repoId={repo.id} worktree={worktree} dir="" depth={0} open={open} onToggle={toggle} active={activePath} onOpen={onOpenFile} />
        </ul>
      ) : (
        <Changes repo={repo} worktree={worktree} active={activePath} onOpen={onOpenDiff} onOpenFile={onOpenFile} />
      )}
    </aside>
  );
}

interface ChangesProps {
  repo: Repo;
  worktree: string | null;
  active: string | null;
  onOpen: (path: string) => void;
  onOpenFile: (path: string) => void;
}

function Changes({ repo, worktree, active, onOpen, onOpenFile }: ChangesProps) {
  const changes = useRepoChanges(repo.id, true, worktree);
  const ask = useConfirm();
  const toast = useToast();
  const revert = (f: FileDiff) => void revertChanges({ repo, worktree, path: f.path, status: f.status, ask, toast });
  const copy = (text: string) =>
    navigator.clipboard.writeText(text).catch((e: unknown) => toast("Couldn't copy", String(e), "danger"));
  const note = (text: string, alert = false) => (
    <p className={`files-note files-changes-note ${alert ? "tr-error" : ""}`} role={alert ? "alert" : undefined}>
      {text}
    </p>
  );
  if (changes.error && !changes.data) return note(`Couldn't read the changes: ${changes.error}`, true);
  if (!changes.data) return note("Loading…");
  if (changes.data.length === 0) return note("No uncommitted changes");
  return (
    <ul className="files-tree files-changes" aria-label="Changed files">
      {changes.data.map((f) => {
        const b = STATUS_BADGE[f.status];
        const slash = f.path.lastIndexOf("/");
        const name = f.path.slice(slash + 1);
        return (
          <ContextMenu.Root key={f.path}>
            <ContextMenu.Trigger asChild>
              <li className="files-item files-change">
                <button
                  type="button"
                  className={`files-row files-file ${active === f.path ? "on" : ""}`}
                  title={f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}
                  aria-current={active === f.path ? "true" : undefined}
                  onClick={() => onOpen(f.path)}
                >
                  <span className={`df-badge ${b.cls}`} aria-label={b.label}>
                    {b.letter}
                  </span>
                  <span className="ellipsis">{name}</span>
                  {slash > 0 && <span className="files-change-dir ellipsis">{f.path.slice(0, slash)}</span>}
                  <span className="files-change-stat num">
                    <span className="diff-add">+{f.additions}</span>
                    {f.deletions > 0 && <span className="diff-del">−{f.deletions}</span>}
                  </span>
                </button>
                <button
                  type="button"
                  className="icon-btn files-revert"
                  aria-label={`Revert changes to ${name}`}
                  title="Revert changes"
                  onClick={() => revert(f)}
                >
                  <svg width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
                    <path d="M3 5h6a3 3 0 0 1 0 6H6" />
                    <path d="M5.5 2.5L3 5l2.5 2.5" />
                  </svg>
                </button>
              </li>
            </ContextMenu.Trigger>
            <ContextMenu.Portal>
              <ContextMenu.Content className="menu tab-menu" collisionPadding={8}>
                <ContextMenu.Item className="menu-item" onSelect={() => onOpen(f.path)}>
                  Open changes
                </ContextMenu.Item>
                <ContextMenu.Item className="menu-item" disabled={f.status === "deleted"} onSelect={() => onOpenFile(f.path)}>
                  Open file
                </ContextMenu.Item>
                <ContextMenu.Separator className="menu-sep" />
                <ContextMenu.Item className="menu-item" onSelect={() => void copy(f.path)}>
                  Copy relative path
                </ContextMenu.Item>
                <ContextMenu.Item className="menu-item" onSelect={() => void copy(`${worktree ?? repo.path}/${f.path}`)}>
                  Copy path
                </ContextMenu.Item>
                <ContextMenu.Separator className="menu-sep" />
                <ContextMenu.Item className="menu-item menu-danger" onSelect={() => revert(f)}>
                  {f.status === "added" ? "Delete new file…" : "Revert changes…"}
                </ContextMenu.Item>
              </ContextMenu.Content>
            </ContextMenu.Portal>
          </ContextMenu.Root>
        );
      })}
    </ul>
  );
}

interface DirProps {
  repoId: string;
  worktree: string | null;
  dir: string;
  depth: number;
  open: ReadonlySet<string>;
  onToggle: (path: string) => void;
  active: string | null;
  onOpen: (path: string) => void;
}

function Dir({ repoId, worktree, dir, depth, open, onToggle, active, onOpen }: DirProps) {
  const entries = useRepoDir(repoId, dir, true, worktree);
  const pad = { paddingLeft: `${8 + depth * 14}px` };
  if (entries.error && !entries.data) {
    return (
      <li className="files-note tr-error" style={pad} role="alert">
        Couldn't list this folder: {entries.error}
      </li>
    );
  }
  if (!entries.data) {
    return (
      <li className="files-note" style={pad}>
        Loading…
      </li>
    );
  }
  if (entries.data.length === 0) {
    return (
      <li className="files-note" style={pad}>
        Empty
      </li>
    );
  }
  return (
    <>
      {entries.data.map((e) => {
        const path = dir ? `${dir}/${e.name}` : e.name;
        if (!e.isDir) {
          return (
            <li key={path} role="treeitem" aria-selected={active === path} className="files-item">
              <button
                type="button"
                className={`files-row files-file ${active === path ? "on" : ""}`}
                style={pad}
                title={path}
                aria-current={active === path ? "true" : undefined}
                onClick={() => onOpen(path)}
              >
                <span className="files-chev" aria-hidden />
                <FileIcon dir={false} />
                <span className="ellipsis">{e.name}</span>
              </button>
            </li>
          );
        }
        const isOpen = open.has(path);
        return (
          <li key={path} role="treeitem" aria-expanded={isOpen} aria-selected={false} className="files-item">
            <button type="button" className="files-row files-dir" style={pad} onClick={() => onToggle(path)}>
              <svg className={`files-chev ${isOpen ? "open" : ""}`} width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
                <path d="M3.5 2l3 3-3 3" />
              </svg>
              <FileIcon dir />
              <span className="ellipsis">{e.name}</span>
            </button>
            {isOpen && (
              <ul role="group">
                <Dir repoId={repoId} worktree={worktree} dir={path} depth={depth + 1} open={open} onToggle={onToggle} active={active} onOpen={onOpen} />
              </ul>
            )}
          </li>
        );
      })}
    </>
  );
}

function FileIcon({ dir }: { dir: boolean }) {
  return (
    <svg className="files-icon" width="13" height="13" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinejoin="round" aria-hidden focusable="false">
      {dir ? <path d="M1.5 3.5A1 1 0 0 1 2.5 2.5h3l1.2 1.5h4.8a1 1 0 0 1 1 1v5.5a1 1 0 0 1-1 1h-9a1 1 0 0 1-1-1z" /> : <path d="M3.5 1.5h4.5l3 3v8h-7.5zM8 1.5v3h3" />}
    </svg>
  );
}

const BRANCH_ICON = (
  <svg className="files-root-icon" width="12" height="12" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" aria-hidden focusable="false">
    <circle cx="4" cy="3" r="1.4" />
    <circle cx="4" cy="11" r="1.4" />
    <circle cx="10" cy="4.5" r="1.4" />
    <path d="M4 4.4v5.2M10 5.9c0 2.4-6 1.8-6 3.4" />
  </svg>
);

/** Which checkout the panel shows: the repo's own or one of its worktrees (name over branch). */
function RootPicker({ repo, worktrees, value, onChange }: { repo: Repo; worktrees: RepoWorktree[]; value: string | null; onChange: (w: string | null) => void }) {
  const current = worktrees.find((w) => (w.isMain ? value === null : w.path === value)) ?? worktrees[0]!;
  const name = (w: RepoWorktree) => (w.isMain ? repo.name : (w.path.split("/").pop() ?? w.path));
  return (
    <div className="files-root">
      <DropdownMenu.Root>
        <DropdownMenu.Trigger className="files-root-btn" aria-label={`Checkout: ${name(current)}`}>
          {BRANCH_ICON}
          <span className="files-root-text">
            <span className="files-root-name ellipsis">{name(current)}</span>
            {current.branch && <span className="files-root-branch ellipsis">{current.branch}</span>}
          </span>
          {!current.isMain && <span className="files-root-tag">worktree</span>}
          <svg className="files-root-caret" width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
            <path d="M2.5 4l2.5 2.5L7.5 4" />
          </svg>
        </DropdownMenu.Trigger>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu tab-menu files-root-menu" align="start" sideOffset={4} collisionPadding={8}>
            <DropdownMenu.RadioGroup value={current.path} onValueChange={(path) => onChange(worktrees.find((w) => w.path === path)?.isMain ? null : path)}>
              {worktrees.map((w, i) => (
                <div key={w.path}>
                  {i === 1 && <DropdownMenu.Label className="menu-label">Worktrees</DropdownMenu.Label>}
                  <DropdownMenu.RadioItem value={w.path} className="menu-item files-root-item" title={w.path}>
                    <span className="menu-mark" aria-hidden>
                      <DropdownMenu.ItemIndicator>✓</DropdownMenu.ItemIndicator>
                    </span>
                    <span className="files-root-text">
                      <span className="files-root-name ellipsis">{name(w)}</span>
                      {w.branch && <span className="files-root-branch ellipsis">{w.branch}</span>}
                    </span>
                  </DropdownMenu.RadioItem>
                </div>
              ))}
            </DropdownMenu.RadioGroup>
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </div>
  );
}
