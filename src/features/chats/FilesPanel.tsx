import { useEffect, useState, type ReactNode } from "react";
import { basename } from "../../domain/hooks/projects";
import { ContextMenu, DropdownMenu } from "radix-ui";
import { rootKey, useRepoChanges, useRepoCommits, useRepoDir, useRepoWorktrees } from "../../domain/hooks/branches";
import { shortAgo } from "./model";
import type { FileDiff } from "../../domain/api";
import { readJsonPref, writePref } from "../../shell/storage";

const isStrings = (v: unknown): v is string[] => Array.isArray(v) && v.every((x) => typeof x === "string");
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { revertChanges } from "./FileView";
import { repoCommit, repoOpenPr } from "../../domain/api";
import { invalidate } from "../../domain/hooks/store";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { RepoWorktree } from "../../domain/api";
import type { DiffFileStatus } from "../../domain/api";
import type { Repo } from "../../domain/types";
import "./diff.css";

interface Props {
  repo: Repo;
  /** The file open in the active tab (relative path), highlighted in the tree. */
  activePath: string | null;
  onOpenFile: (path: string) => void;
  /** Opens a changed file's diff (from the "Changes" view). */
  onOpenDiff: (path: string) => void;
  /** Asks the active chat to open a PR for this branch; `null` when no chat is open. */
  onAskChatForPr: (() => void) | null;
  /** Opens a commit's changes in a tab; `activeCommit` is the one in the active tab. */
  onOpenCommit: (sha: string) => void;
  activeCommit: string | null;
  /** All files, or only the uncommitted changes (the chat's "N files changed" opens this). */
  mode: FilesMode;
  onMode: (mode: FilesMode) => void;
  /** The checkout shown: one of the repo's worktrees (absolute path), `null` for the repo's own. */
  worktree: string | null;
  onWorktree: (worktree: string | null) => void;
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
export function FilesPanel({ repo, activePath, onOpenFile, onOpenDiff, onOpenCommit, activeCommit, onAskChatForPr, mode, onMode: setMode, worktree, onWorktree, onClose }: Props) {
  const worktrees = useRepoWorktrees(repo.id).data;
  // Files picked in "Changes" (⇧/⌘-click) for the next commit, tied to the checkout they're in.
  const [picked, setPicked] = useState<Picked>({ root: "", paths: [], anchor: null });
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
        {/* Files or git, as one capsule split in two: the same shape as the composer's settings. */}
        <div className="files-modes" role="group" aria-label={`${repo.name}: show`} title={repo.path}>
          <button type="button" className={`files-mode ${mode === "tree" ? "on" : ""}`} aria-pressed={mode === "tree"} aria-label="Files" title="Files" onClick={() => setMode("tree")}>
            <FileIcon dir />
          </button>
          <button type="button" className={`files-mode ${mode === "changes" ? "on" : ""}`} aria-pressed={mode === "changes"} aria-label="Git" title="Changes and commits (git)" onClick={() => setMode("changes")}>
            <svg width="13" height="13" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" aria-hidden focusable="false">
              <circle cx="4" cy="3" r="1.5" />
              <circle cx="4" cy="11" r="1.5" />
              <circle cx="10" cy="5" r="1.5" />
              <path d="M4 4.5v5M10 6.5c0 2-2 2.5-6 3" />
            </svg>
          </button>
        </div>
        <button type="button" className="icon-btn files-close" aria-label="Close files" title="Close" onClick={onClose}>
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
        <div className="files-sections">
          <GitBox
            key={rootKey(repo.id, worktree)}
            repo={repo}
            worktree={worktree}
            picked={picked.root === rootKey(repo.id, worktree) ? picked.paths : []}
            onCommitted={() => setPicked({ root: "", paths: [], anchor: null })}
            onAskChatForPr={onAskChatForPr}
          />
          <Section id="changes" title="Changes">
            <Changes
              repo={repo}
              worktree={worktree}
              active={activePath}
              picked={picked.root === rootKey(repo.id, worktree) ? picked : null}
              onPick={(paths, anchor) => setPicked({ root: rootKey(repo.id, worktree), paths, anchor })}
              onOpen={onOpenDiff}
              onOpenFile={onOpenFile}
            />
          </Section>
          <Section id="commits" title={worktree ? "Commits on this branch" : "Commits"}>
            <Commits repo={repo} worktree={worktree} active={activeCommit} onOpen={onOpenCommit} />
          </Section>
        </div>
      )}
    </aside>
  );
}

interface Picked {
  root: string;
  paths: string[];
  anchor: string | null;
}

interface ChangesProps {
  repo: Repo;
  worktree: string | null;
  active: string | null;
  picked: Picked | null;
  onPick: (paths: string[], anchor: string | null) => void;
  onOpen: (path: string) => void;
  onOpenFile: (path: string) => void;
}

function Changes({ repo, worktree, active, picked, onPick, onOpen, onOpenFile }: ChangesProps) {
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
  if (changes.data.length === 0) return note(worktree ? "No changes on this branch" : "No uncommitted changes");
  const sel = new Set((picked?.paths ?? []).filter((p) => changes.data!.some((f) => f.path === p)));
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
                  className={`files-row files-file ${active === f.path ? "on" : ""} ${sel.has(f.path) ? "picked" : ""}`}
                  title={f.oldPath ? `${f.oldPath} → ${f.path}` : `${f.path}\n⇧-click: pick a range · ⌘-click: pick this one`}
                  aria-current={active === f.path ? "true" : undefined}
                  aria-pressed={sel.size > 0 ? sel.has(f.path) : undefined}
                  onClick={(e) => {
                    const all = changes.data!.map((x) => x.path);
                    if (e.shiftKey) {
                      // A range from the last picked file (or the first) to this one.
                      const from = all.indexOf(picked?.anchor ?? all[0]!);
                      const to = all.indexOf(f.path);
                      onPick(all.slice(Math.min(from, to), Math.max(from, to) + 1), picked?.anchor ?? all[0]!);
                    } else if (e.metaKey || e.ctrlKey) {
                      onPick(sel.has(f.path) ? [...sel].filter((p) => p !== f.path) : [...sel, f.path], f.path);
                    } else {
                      onPick([], f.path);
                      onOpen(f.path);
                    }
                  }}
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

/** Which checkout the panel shows: the repo's own or one of its worktrees, each by its branch. */
function RootPicker({ repo, worktrees, value, onChange }: { repo: Repo; worktrees: RepoWorktree[]; value: string | null; onChange: (w: string | null) => void }) {
  const current = worktrees.find((w) => (w.isMain ? value === null : w.path === value)) ?? worktrees[0]!;
  const name = (w: RepoWorktree) => (w.isMain ? repo.name : basename(w.path));
  // A worktree's branch already carries its name; a detached checkout falls back to the folder.
  const label = (w: RepoWorktree) => w.branch ?? name(w);
  const tag = <span className="files-root-tag">main repo</span>;
  return (
    <div className={`files-root ${current.isMain ? "" : "wt"}`}>
      <DropdownMenu.Root>
        <DropdownMenu.Trigger className="files-root-btn" aria-label={`Checkout: ${label(current)}`} title={`${name(current)}\n${current.path}`}>
          {BRANCH_ICON}
          <span className="files-root-name ellipsis">{label(current)}</span>
          {current.isMain && tag}
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
                  <DropdownMenu.RadioItem value={w.path} className="menu-item files-root-item" title={`${name(w)}\n${w.path}`}>
                    <span className="menu-mark" aria-hidden>
                      <DropdownMenu.ItemIndicator>✓</DropdownMenu.ItemIndicator>
                    </span>
                    <span className="files-root-name ellipsis">{label(w)}</span>
                    {w.isMain && tag}
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

/** A collapsible block of the git view (like VS Code's Source Control sections); open state remembered. */
function Section({ id, title, children }: { id: string; title: string; children: ReactNode }) {
  const key = `filesSection:${id}`;
  const [open, setOpen] = useState(() => readJsonPref(key, true, (v): v is boolean => typeof v === "boolean"));
  return (
    <section className={`files-section ${open ? "open" : ""}`}>
      <button
        type="button"
        className="files-section-head"
        aria-expanded={open}
        onClick={() => {
          writePref(key, JSON.stringify(!open));
          setOpen(!open);
        }}
      >
        <span className={`files-chev ${open ? "open" : ""}`} aria-hidden>
          ›
        </span>
        {title}
      </button>
      {open && <div className="files-section-body">{children}</div>}
    </section>
  );
}

/** The checkout's commits, newest first; one opens its changes in a tab. */
function Commits({ repo, worktree, active, onOpen }: { repo: Repo; worktree: string | null; active: string | null; onOpen: (sha: string) => void }) {
  const commits = useRepoCommits(repo.id, true, worktree);
  const now = Date.now();
  if (commits.error && !commits.data) {
    return (
      <p className="files-note files-changes-note tr-error" role="alert">
        Couldn't read the commits: {commits.error}
      </p>
    );
  }
  if (!commits.data) return <p className="files-note files-changes-note">Loading…</p>;
  if (commits.data.length === 0) {
    return <p className="files-note files-changes-note">{worktree ? "No commits on this branch yet" : "No commits yet"}</p>;
  }
  return (
    <ul className="files-tree files-changes" aria-label="Commits">
      {commits.data.map((c) => (
        <li key={c.sha} className="files-item">
          <button
            type="button"
            className={`files-row files-commit ${active === c.sha ? "on" : ""}`}
            title={`${c.subject}\n${c.shortSha} · ${c.author}`}
            aria-current={active === c.sha ? "true" : undefined}
            onClick={() => onOpen(c.sha)}
          >
            <span className="files-commit-dot" aria-hidden />
            <span className="ellipsis">{c.subject}</span>
            <span className="files-change-dir num">{shortAgo(c.at, now)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/** VS Code-style commit box, plus a pull request: opened with the GitHub CLI or asked of the chat. */
function GitBox({
  repo,
  worktree,
  picked,
  onCommitted,
  onAskChatForPr,
}: {
  repo: Repo;
  worktree: string | null;
  /** Files picked in "Changes": commit only those; none → everything. */
  picked: string[];
  onCommitted: () => void;
  onAskChatForPr: (() => void) | null;
}) {
  const toast = useToast();
  const changes = useRepoChanges(repo.id, true, worktree).data;
  const latest = useRepoCommits(repo.id, true, worktree).data?.[0];
  const [message, setMessage] = useState("");
  const [busy, setBusy] = useState<"commit" | "pr" | null>(null);
  const [pr, setPr] = useState<{ title: string; body: string } | null>(null);
  // In a worktree, "changes" also counts committed work; the box commits what's on disk.
  const files = picked.length || (changes?.length ?? 0);
  const canCommit = !!message.trim() && files > 0 && !busy;

  const commit = async () => {
    if (!canCommit) return;
    setBusy("commit");
    try {
      const c = await repoCommit(repo.id, worktree, message.trim(), picked);
      setMessage("");
      onCommitted();
      toast("Committed", `${c.shortSha} ${c.subject}`, "ok");
    } catch (e) {
      toast("Couldn't commit", String(e), "danger");
    } finally {
      setBusy(null);
      void invalidate("chats");
    }
  };

  const openPr = async () => {
    if (!pr || !pr.title.trim()) return;
    setBusy("pr");
    try {
      const url = await repoOpenPr(repo.id, worktree, pr.title.trim(), pr.body);
      setPr(null);
      toast("Pull request ready", url, "ok");
      openUrl(url).catch(() => {});
    } catch (e) {
      toast("Couldn't open the pull request", String(e), "danger");
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="git-box">
      <label className="sr-only" htmlFor={`git-msg-${repo.id}`}>
        Commit message
      </label>
      <textarea
        id={`git-msg-${repo.id}`}
        className="git-msg"
        rows={2}
        placeholder={files > 0 ? "Commit message (⌘↵ to commit)" : "Nothing to commit"}
        value={message}
        disabled={files === 0 && !message}
        onChange={(e) => setMessage(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            void commit();
          }
        }}
      />
      <div className="git-actions">
        <button type="button" className="btn btn-sm btn-primary git-commit" disabled={!canCommit} onClick={() => void commit()}>
          {busy === "commit"
            ? "Committing…"
            : picked.length > 0
              ? `Commit ${picked.length} picked`
              : files > 0
                ? `Commit all ${files} ${files === 1 ? "file" : "files"}`
                : "Commit"}
        </button>
        <DropdownMenu.Root>
          <DropdownMenu.Trigger className="btn btn-sm git-pr" disabled={!!busy || !worktree} title={worktree ? undefined : "Open a PR from a worktree branch"}>
            Pull request
            <svg width="9" height="9" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
              <path d="M2.5 4l2.5 2.5L7.5 4" />
            </svg>
          </DropdownMenu.Trigger>
          <DropdownMenu.Portal>
            <DropdownMenu.Content className="menu tab-menu" align="end" sideOffset={4} collisionPadding={8}>
              <DropdownMenu.Item className="menu-item" onSelect={() => setPr({ title: latest?.subject ?? "", body: "" })}>
                Open with GitHub CLI…
              </DropdownMenu.Item>
              <DropdownMenu.Item className="menu-item" disabled={!onAskChatForPr} onSelect={() => onAskChatForPr?.()}>
                Ask the chat to open it
              </DropdownMenu.Item>
            </DropdownMenu.Content>
          </DropdownMenu.Portal>
        </DropdownMenu.Root>
      </div>
      {pr && (
        <form
          className="git-pr-form"
          onSubmit={(e) => {
            e.preventDefault();
            void openPr();
          }}
        >
          <label className="git-label" htmlFor={`git-pr-title-${repo.id}`}>
            Title
          </label>
          <input id={`git-pr-title-${repo.id}`} className="input git-input" value={pr.title} onChange={(e) => setPr({ ...pr, title: e.target.value })} autoFocus />
          <label className="git-label" htmlFor={`git-pr-body-${repo.id}`}>
            Description
          </label>
          <textarea id={`git-pr-body-${repo.id}`} className="git-msg" rows={4} value={pr.body} onChange={(e) => setPr({ ...pr, body: e.target.value })} />
          <div className="git-actions">
            <button type="submit" className="btn btn-sm btn-primary" disabled={!pr.title.trim() || !!busy}>
              {busy === "pr" ? "Pushing and opening…" : "Push and open PR"}
            </button>
            <button type="button" className="btn btn-sm btn-ghost" disabled={!!busy} onClick={() => setPr(null)}>
              Cancel
            </button>
          </div>
        </form>
      )}
    </div>
  );
}
