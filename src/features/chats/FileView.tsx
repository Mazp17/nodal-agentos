import { useMemo, useState, type KeyboardEvent } from "react";
import { basename } from "../../domain/hooks/projects";
import { revertRepoFile, writeRepoFile } from "../../domain/api";
import { repoFileKey, useCommitDiff, useRepoChanges, useRepoCommits, useRepoFile } from "../../domain/hooks/branches";
import { invalidate, setData } from "../../domain/hooks/store";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { getDraft, setDraft, useDirtyTabs } from "./drafts";
import { DiffLines } from "./DiffLines";
import type { DiffFileStatus } from "../../domain/api";
import type { Repo } from "../../domain/types";
import { extOf, languageOf, tokenize } from "./highlight";

interface Props {
  repo: Repo;
  /** One of the repo's worktrees (absolute path); `null` is the repo's own checkout. */
  worktree: string | null;
  path: string;
}

/** Tokenized lines; past the cap they stay plain (still editable). */
const MAX_HIGHLIGHT = 5000;

interface EditProps extends Props {
  /** Tab id: unsaved edits are kept per tab (`drafts`). */
  tab: string;
}

/** A repo file opened from the files panel: editable text with line numbers and highlighting.
 * Binary and truncated (> 1 MB) files are read only. ⌘S saves; a file changed on disk since it
 * was opened asks to reload or overwrite. */
export function FileView({ repo, worktree, path, tab }: EditProps) {
  const toast = useToast();
  const file = useRepoFile(repo.id, path, worktree);
  const dirtyTabs = useDirtyTabs();
  const draft = dirtyTabs.has(tab) ? getDraft(tab) : undefined;
  const [saving, setSaving] = useState(false);
  const [conflict, setConflict] = useState(false);
  /** The caret's line while the editor has focus, highlighted like VS Code's current line. */
  const [caretLine, setCaretLine] = useState<number | null>(null);
  const ext = extOf(path);
  const slash = path.lastIndexOf("/");
  const data = file.data;
  const editable = !!data && !data.binary && !data.truncated;
  const text = draft?.text ?? data?.content ?? "";

  const lines = useMemo(() => {
    const all = text.split("\n");
    return all.map((l, i) => (i < MAX_HIGHLIGHT ? tokenize(l, ext) : [{ t: l, k: "txt" as const }]));
  }, [text, ext]);

  const edit = (next: string) => {
    if (!data) return;
    if (next === data.content) setDraft(tab, null);
    else setDraft(tab, { text: next, version: draft?.version ?? data.version });
  };

  const save = async (force = false) => {
    const d = getDraft(tab);
    if (!d || saving) return;
    setSaving(true);
    try {
      const saved = await writeRepoFile(repo.id, path, d.text, d.version, force, worktree);
      setDraft(tab, null);
      setConflict(false);
      setData(repoFileKey(repo.id, path, worktree), () => saved);
      // The chat's "N files changed" and the files panel's git view.
      void invalidate("chats", "repos");
    } catch (e) {
      const msg = String(e);
      if (msg.includes("File changed on disk")) setConflict(true);
      else toast("Couldn't save the file", msg, "danger");
    } finally {
      setSaving(false);
    }
  };

  const reload = () => {
    setDraft(tab, null);
    setConflict(false);
    void invalidate("repos");
  };

  const onKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "s") {
      e.preventDefault();
      void save();
    } else if (e.key === "Tab" && !e.metaKey && !e.ctrlKey && !e.altKey && !e.shiftKey) {
      // Indent instead of leaving the editor (Esc then Tab still moves focus).
      e.preventDefault();
      document.execCommand("insertText", false, "  ");
    }
  };

  return (
    <section className="file-view" aria-label={editable ? path : `${path} (read only)`}>
      <header className="file-view-head">
        <p className="file-view-path ellipsis" title={`${worktree ?? repo.path}/${path}`}>
          <RootCrumb repo={repo} worktree={worktree} />
          {slash >= 0 && <span>{path.slice(0, slash + 1)}</span>}
          <span className="file-view-name">{path.slice(slash + 1)}</span>
        </p>
        {data && !data.binary && (
          <span className="file-view-meta">
            {languageOf(path)} · {lines.length.toLocaleString()} {lines.length === 1 ? "line" : "lines"}
          </span>
        )}
        {editable ? (
          <>
            {draft && <span className="file-view-dirty">Unsaved</span>}
            <button type="button" className="btn btn-sm" disabled={!draft || saving} aria-keyshortcuts="Meta+S" onClick={() => void save()}>
              {saving ? "Saving…" : "Save"}
            </button>
          </>
        ) : (
          data && <span className="file-view-badge">Read only</span>
        )}
      </header>
      {conflict && (
        <div className="banner banner-warn file-view-conflict" role="alert">
          <span>This file changed on disk since you opened it.</span>
          <button type="button" className="btn btn-sm" onClick={reload}>
            Reload (discard my edits)
          </button>
          <button type="button" className="btn btn-sm" onClick={() => void save(true)}>
            Overwrite
          </button>
        </div>
      )}
      {file.error && !data ? (
        <p className="file-view-note tr-error" role="alert">
          Couldn't open this file: {file.error}
        </p>
      ) : !data ? (
        <p className="file-view-note">Loading…</p>
      ) : data.binary ? (
        <p className="file-view-note">This is a binary file; it can't be shown as text.</p>
      ) : (
        <div className="file-view-code">
          <div className="file-edit">
            <div className="file-edit-gutter" aria-hidden>
              {lines.map((_, i) => (
                <div key={i} className={i === caretLine ? "on" : undefined}>
                  {i + 1}
                </div>
              ))}
            </div>
            <div className="file-edit-stack">
              <pre className="file-edit-hl" aria-hidden>
                {lines.map((tokens, i) => (
                  <div key={i} className={i === caretLine ? "file-edit-line on" : "file-edit-line"}>
                    {tokens.length === 0
                      ? "​"
                      : tokens.map((tk, j) => (
                          <span key={j} className={`tok-${tk.k}`}>
                            {tk.t}
                          </span>
                        ))}
                  </div>
                ))}
              </pre>
              {editable && (
                <textarea
                  className="file-edit-input"
                  aria-label={`Contents of ${path}`}
                  value={text}
                  spellCheck={false}
                  autoCapitalize="off"
                  autoCorrect="off"
                  wrap="off"
                  onChange={(e) => edit(e.target.value)}
                  onKeyDown={onKeyDown}
                  onSelect={(e) => {
                    const t = e.currentTarget;
                    setCaretLine(t.value.slice(0, t.selectionEnd).split("\n").length - 1);
                  }}
                  onBlur={() => setCaretLine(null)}
                />
              )}
            </div>
          </div>
          {data.truncated && (
            <p className="file-view-note">
              Showing the first 1 MB of a {(data.size / 1_048_576).toFixed(1)} MB file; it can't be edited here.
            </p>
          )}
        </div>
      )}
    </section>
  );
}

/** A changed file's uncommitted diff vs HEAD, opened from the files panel's "Changes" view. */
export function FileDiffView({ repo, worktree, path }: Props) {
  const toast = useToast();
  const ask = useConfirm();
  const changes = useRepoChanges(repo.id, true, worktree);
  const file = changes.data?.find((f) => f.path === path);
  const slash = path.lastIndexOf("/");
  return (
    <section className="file-view" aria-label={`Changes in ${path}`}>
      <header className="file-view-head">
        <p className="file-view-path ellipsis" title={file?.oldPath ? `${file.oldPath} → ${path}` : `${worktree ?? repo.path}/${path}`}>
          <RootCrumb repo={repo} worktree={worktree} />
          {slash >= 0 && <span>{path.slice(0, slash + 1)}</span>}
          <span className="file-view-name">{path.slice(slash + 1)}</span>
        </p>
        {file && (
          <span className="file-view-meta num">
            <span className="diff-add">+{file.additions}</span> <span className="diff-del">−{file.deletions}</span>
          </span>
        )}
        <span className="file-view-badge">{worktree ? "vs base" : "vs HEAD"}</span>
        {file && (
          <button
            type="button"
            className="btn btn-sm"
            onClick={() => void revertChanges({ repo, worktree, path, status: file.status, ask, toast })}
          >
            Revert
          </button>
        )}
      </header>
      {changes.error && !changes.data ? (
        <p className="file-view-note tr-error" role="alert">
          Couldn't read the changes: {changes.error}
        </p>
      ) : !changes.data ? (
        <p className="file-view-note">Loading…</p>
      ) : !file ? (
        <p className="file-view-note">{worktree ? "No changes to this file on this branch." : "No uncommitted changes in this file."}</p>
      ) : (
        <div className="file-view-code" role="region" aria-label={`Diff of ${path}`} tabIndex={0}>
          <DiffLines file={file} />
        </div>
      )}
    </section>
  );
}

/** The project's repo first (that's what the file belongs to), then the worktree it's read
 * from, if any, as a tag: `nodal-playground [worktree-name] / src/cart.js`. */
function RootCrumb({ repo, worktree }: { repo: Repo; worktree: string | null }) {
  return (
    <>
      <span className="file-view-repo">{repo.name}</span>
      {worktree && (
        <span className="file-view-wt" title={`Worktree: ${worktree}`}>
          <svg width="10" height="10" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" aria-hidden focusable="false">
            <circle cx="4" cy="3" r="1.4" />
            <circle cx="4" cy="11" r="1.4" />
            <circle cx="10" cy="4.5" r="1.4" />
            <path d="M4 4.4v5.2M10 5.9c0 2.4-6 1.8-6 3.4" />
          </svg>
          <span className="sr-only">worktree </span>
          {basename(worktree)}
        </span>
      )}
      <span className="file-view-repo">/</span>
    </>
  );
}

/** Throws away a file's uncommitted changes after asking (a new file is deleted). */
export async function revertChanges(a: {
  repo: Repo;
  worktree: string | null;
  path: string;
  status: DiffFileStatus;
  ask: ReturnType<typeof useConfirm>;
  toast: ReturnType<typeof useToast>;
}): Promise<boolean> {
  const name = a.path.split("/").pop() ?? a.path;
  const ok = await a.ask({
    title: a.status === "added" ? `Delete the new file ${name}?` : `Revert the changes to ${name}?`,
    body:
      a.status === "added"
        ? "It isn't in the last commit, so reverting deletes it. This can't be undone."
        : "It goes back to how it is in the last commit. Uncommitted edits are lost; this can't be undone.",
    confirmLabel: a.status === "added" ? "Delete file" : "Revert",
  });
  if (!ok) return false;
  try {
    await revertRepoFile(a.repo.id, a.path, a.worktree);
    void invalidate("repos", "chats");
    return true;
  } catch (e) {
    a.toast("Couldn't revert the file", String(e), "danger");
    return false;
  }
}

/** One commit's changes: every file it touched, stacked. */
export function CommitView({ repo, worktree, sha }: { repo: Repo; worktree: string | null; sha: string }) {
  const files = useCommitDiff(repo.id, sha, worktree);
  const info = useRepoCommits(repo.id, true, worktree).data?.find((c) => c.sha === sha);
  const adds = files.data?.reduce((n, f) => n + f.additions, 0) ?? 0;
  const dels = files.data?.reduce((n, f) => n + f.deletions, 0) ?? 0;
  return (
    <section className="file-view" aria-label={`Commit ${sha.slice(0, 7)}`}>
      <header className="file-view-head">
        <p className="file-view-path ellipsis" title={`${worktree ?? repo.path} @ ${sha}`}>
          <RootCrumb repo={repo} worktree={worktree} />
          <span className="file-view-name">{info?.subject ?? sha.slice(0, 7)}</span>
        </p>
        {files.data && (
          <span className="file-view-meta num">
            {files.data.length} {files.data.length === 1 ? "file" : "files"} <span className="diff-add">+{adds}</span> <span className="diff-del">−{dels}</span>
          </span>
        )}
        <span className="file-view-badge mono" title={info ? `${info.author}` : undefined}>
          {sha.slice(0, 7)}
        </span>
      </header>
      {files.error && !files.data ? (
        <p className="file-view-note tr-error" role="alert">
          Couldn't read the commit: {files.error}
        </p>
      ) : !files.data ? (
        <p className="file-view-note">Loading…</p>
      ) : files.data.length === 0 ? (
        <p className="file-view-note">This commit changes no files.</p>
      ) : (
        <div className="file-view-code commit-files" role="region" aria-label="Changed files" tabIndex={0}>
          {files.data.map((f) => (
            <div key={f.path} className="commit-file">
              <div className="commit-file-head">
                <span className="ellipsis">{f.oldPath ? `${f.oldPath} → ${f.path}` : f.path}</span>
                <span className="num">
                  <span className="diff-add">+{f.additions}</span> <span className="diff-del">−{f.deletions}</span>
                </span>
              </div>
              <DiffLines file={f} />
            </div>
          ))}
        </div>
      )}
    </section>
  );
}
