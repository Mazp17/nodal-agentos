import { useContext, useState } from "react";
import { createChat, removeRepoWorktree, type RepoWorktree } from "../../domain/api";
import { useRepoWorktrees } from "../../domain/hooks/branches";
import { useAllChats } from "../../domain/hooks/chats";
import { useProjects } from "../../domain/hooks/projects";
import { invalidate, setData, useTasks } from "../../domain/hooks/store";
import type { Chat, Project, Repo, Task } from "../../domain/types";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { SectionHead } from "../settings/SettingsView";
import { taskTag } from "../tasks/status";
import { OpenChatContext } from "../tasks/useTaskChat";

const withChat = (cs: Chat[], c: Chat) => (cs.some((x) => x.id === c.id) ? cs : [c, ...cs]);

/** The project's git worktrees, per repo: what each is for, a chat inside it, or delete it. */
export function WorktreesSection({ project }: { project: Project }) {
  const ctx = useProjects();
  const repos = ctx.reposOf(project.id);
  const tasks = useTasks(project.id).data ?? [];
  const chats = useAllChats(true).data ?? [];
  return (
    <>
      <SectionHead
        title="Worktrees"
        text="Separate checkouts of this project's repos, each on its own branch. Task chats make one per task; Claude can make its own. Open a chat inside one to ask about that work, or delete it."
      />
      {repos.length === 0 && <p className="settings-note">This project has no repos yet.</p>}
      {repos.map((r) => (
        <RepoWorktrees key={r.id} project={project} repo={r} tasks={tasks} chats={chats} />
      ))}
    </>
  );
}

function RepoWorktrees({ project, repo, tasks, chats }: { project: Project; repo: Repo; tasks: Task[]; chats: Chat[] }) {
  const list = useRepoWorktrees(repo.id);
  const wts = (list.data ?? []).filter((w) => !w.isMain);
  return (
    <section className="panel settings-card wt-group" aria-label={repo.name}>
      <header className="aw-group-head">
        <span className="aw-group-title">{repo.name}</span>
        <span className="aw-group-hint mono ellipsis" title={repo.path}>
          {repo.path}
        </span>
        <span className="aw-group-count">{list.data ? `${wts.length} ${wts.length === 1 ? "worktree" : "worktrees"}` : ""}</span>
      </header>
      {list.error && !list.data ? (
        <p className="wt-note tr-error" role="alert">
          Couldn't list the worktrees: {list.error}
        </p>
      ) : !list.data ? (
        <p className="wt-note">Loading…</p>
      ) : wts.length === 0 ? (
        <p className="wt-note">No worktrees. A task opened in a chat gets one.</p>
      ) : (
        <ul className="wt-list">
          {wts.map((w) => (
            <WorktreeRow
              key={w.path}
              project={project}
              repo={repo}
              wt={w}
              task={tasks.find((t) => t.repoId === repo.id && t.worktree?.path === w.path) ?? null}
              chats={chats.filter((c) => c.worktree === w.path).length}
            />
          ))}
        </ul>
      )}
    </section>
  );
}

function WorktreeRow({ project, repo, wt, task, chats }: { project: Project; repo: Repo; wt: RepoWorktree; task: Task | null; chats: number }) {
  const open = useContext(OpenChatContext);
  const ask = useConfirm();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const name = wt.path.split("/").pop() ?? wt.path;

  const chatHere = async () => {
    setBusy(true);
    try {
      const chat = await createChat(project.id, { repoId: repo.id, worktree: wt.path });
      setData<Chat[]>(`chats:${project.id}`, (cs) => withChat(cs, chat));
      setData<Chat[]>("chats:*", (cs) => withChat(cs, chat));
      void invalidate("chats");
      open?.(chat);
    } catch (e) {
      toast("Couldn't open a chat there", String(e), "danger");
    } finally {
      setBusy(false);
    }
  };

  const remove = async () => {
    const ok = await ask({
      title: `Delete ${name}?`,
      body: `Removes the folder${wt.branch ? ` and the branch ${wt.branch}` : ""}.${task ? ` ${taskTag(task, project.key)} loses its worktree.` : ""}`,
      confirmLabel: "Delete worktree",
    });
    if (!ok) return;
    setBusy(true);
    try {
      try {
        await removeRepoWorktree(repo.id, wt.path, false);
      } catch (e) {
        // Unsaved work in it: say what would be lost, delete only on a second yes.
        const force = await ask({
          title: `${name} has work that isn't saved elsewhere`,
          body: `${String(e)} Delete it anyway? This can't be undone.`,
          confirmLabel: "Delete anyway",
        });
        if (!force) return;
        await removeRepoWorktree(repo.id, wt.path, true);
      }
      toast("Worktree deleted", name, "ok");
    } catch (e) {
      toast("Couldn't delete the worktree", String(e), "danger");
    } finally {
      setBusy(false);
      void invalidate("repos", "tasks", "chats");
    }
  };

  return (
    <li className="wt-row">
      <svg className="wt-icon" width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" aria-hidden focusable="false">
        <circle cx="4" cy="3" r="1.4" />
        <circle cx="4" cy="11" r="1.4" />
        <circle cx="10" cy="4.5" r="1.4" />
        <path d="M4 4.4v5.2M10 5.9c0 2.4-6 1.8-6 3.4" />
      </svg>
      <div className="wt-text">
        <span className="wt-name ellipsis">{name}</span>
        <span className="wt-meta ellipsis" title={wt.path}>
          <span className="mono">{wt.branch ?? "detached HEAD"}</span>
          {task && (
            <>
              <span aria-hidden> · </span>
              <span className="wt-task">
                {taskTag(task, project.key)} {task.title}
              </span>
            </>
          )}
          {chats > 0 && (
            <>
              <span aria-hidden> · </span>
              {chats} {chats === 1 ? "chat" : "chats"}
            </>
          )}
        </span>
      </div>
      <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void chatHere()}>
        Open chat
      </button>
      <button type="button" className="btn btn-sm btn-ghost wt-delete" disabled={busy} onClick={() => void remove()}>
        Delete
      </button>
    </li>
  );
}
