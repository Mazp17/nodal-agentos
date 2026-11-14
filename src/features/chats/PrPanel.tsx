import { useRef, useState, type ReactNode } from "react";
import { DropdownMenu } from "radix-ui";
import { chatPrs, repoCheckLog, repoLinkPr, repoMergePr, repoPr, repoPrDetails, repoUnlinkPr, type BranchPr, type MergeMethod, type PrCheck, type PrDetails } from "../../domain/api";
import { useConfirm } from "../../ui/ConfirmDialog";
import { rootKey } from "../../domain/hooks/branches";
import { usePolled } from "../../domain/hooks/store";
import type { Repo } from "../../domain/types";
import { ExternalLink } from "../../ui/ExternalLink";
import { SafeMarkdown } from "../../ui/Markdown";
import { useToast } from "../../ui/Toasts";
import { shortAgo } from "./model";
import { sendToChat } from "./stream";
import { useWorktreeTask } from "./useWorktreeTask";

const FAILED = new Set(["FAILURE", "TIMED_OUT", "CANCELLED", "ACTION_REQUIRED", "STARTUP_FAILURE"]);
const DETAILS_POLL = 30_000;

type PrState = BranchPr["state"];
type CheckState = "failed" | "pending" | "passed";

interface Row {
  name: string;
  url: string;
  state: CheckState;
}

const checkRow = (c: PrCheck): Row =>
  c.__typename === "CheckRun"
    ? { name: c.workflowName && c.workflowName !== c.name ? `${c.workflowName} / ${c.name}` : c.name, url: c.detailsUrl, state: c.status !== "COMPLETED" ? "pending" : c.conclusion && FAILED.has(c.conclusion) ? "failed" : "passed" }
    : { name: c.context, url: c.targetUrl, state: c.state === "PENDING" ? "pending" : c.state === "FAILURE" || c.state === "ERROR" ? "failed" : "passed" };

const ORDER: Record<CheckState, number> = { failed: 0, pending: 1, passed: 2 };
const STATE_LABEL: Record<PrState, string> = { open: "Open", draft: "Draft", merged: "Merged", closed: "Closed" };
const DECISION: Record<string, { label: string; tone: string }> = {
  APPROVED: { label: "Approved", tone: "ok" },
  CHANGES_REQUESTED: { label: "Changes requested", tone: "danger" },
  REVIEW_REQUIRED: { label: "Review required", tone: "muted" },
};

interface Candidate {
  number: number;
  url: string;
  branch: string | null;
  /** The checkout branch's own PR (from `repoPr`), not just one the chat opened or linked. */
  current: BranchPr | null;
}

/** Every PR of the checkout and the chat. Open ones show everything (checks, review, inline
 * comments) without a click; merged and closed ones fold to one line. */
export function PrPanel({ repo, worktree, chatId, onAskChatForPr }: { repo: Repo; worktree: string | null; chatId: string | null; onAskChatForPr: (() => void) | null }) {
  const key = rootKey(repo.id, worktree);
  const force = useRef(false);
  const pr = usePolled(`repo-pr:${key}`, () => repoPr(repo.id, worktree, force.current), ["repos"], 0);
  const mine = usePolled(chatId ? `chat-prs:${chatId}` : null, () => chatPrs(chatId!), ["chats"], 0);
  const [linking, setLinking] = useState(false);
  // Bumped by Refresh: each PR re-reads its details.
  const [tick, setTick] = useState(0);
  const toast = useToast();
  const { chat: taskChat } = useWorktreeTask(repo, worktree);
  const target = chatId ?? taskChat?.id ?? null;
  const branchPr = pr.data ?? null;
  // The branch PR first, then the chat's PRs of this repo, newest first, deduped by number.
  const cands: Candidate[] = [
    ...(branchPr ? [{ number: branchPr.number, url: branchPr.url, branch: branchPr.branch, current: branchPr }] : []),
    ...(mine.data ?? []).filter((c) => c.repoId === repo.id && c.number !== branchPr?.number).map((c) => ({ number: c.number, url: c.url, branch: c.branch, current: null })),
  ];

  const refresh = async () => {
    force.current = true;
    try {
      await pr.refresh();
    } finally {
      force.current = false;
    }
    await mine.refresh();
    setTick((t) => t + 1);
  };
  const onLinked = () => {
    setLinking(false);
    void pr.refresh();
    void mine.refresh();
  };

  if (pr.error && !pr.data) {
    return (
      <p className="files-note files-changes-note tr-error" role="alert">
        Couldn't read the pull requests: {pr.error}
      </p>
    );
  }
  if (pr.data === undefined) return <p className="files-note files-changes-note">Loading…</p>;
  if (cands.length === 0) {
    return (
      <div className="files-sections">
        <p className="files-note files-changes-note">{worktree ? "No pull request for this branch yet." : "No pull request for this checkout yet."}</p>
        <LinkForm repo={repo} worktree={worktree} chatId={chatId} onLinked={onLinked} onAskChatForPr={onAskChatForPr} />
      </div>
    );
  }

  return (
    <div className="files-sections pr-panel">
      <div className="pr-bar">
        <span className="pr-bar-count">
          {cands.length} pull {cands.length === 1 ? "request" : "requests"}
        </span>
        <button type="button" className="btn btn-sm btn-ghost" aria-expanded={linking} onClick={() => setLinking((v) => !v)}>
          Link PR
        </button>
        <button
          type="button"
          className="icon-btn"
          title="Refresh"
          aria-label="Refresh the pull requests"
          onClick={() => void refresh().catch((e: unknown) => toast("Couldn't refresh", String(e), "danger"))}
        >
          <svg width="13" height="13" viewBox="0 0 16 16" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
            <path d="M13.5 8a5.5 5.5 0 1 1-1.6-3.9M13.5 2.5v3h-3" />
          </svg>
        </button>
      </div>
      {linking && <LinkForm repo={repo} worktree={worktree} chatId={chatId} onLinked={onLinked} onAskChatForPr={null} />}
      <ul className="pr-list" aria-label="Pull requests">
        {cands.map((c) => (
          <PrItem key={c.number} repo={repo} worktree={worktree} cand={c} tick={tick} target={target} onUnlinked={() => void pr.refresh()} />
        ))}
      </ul>
    </div>
  );
}

/** One PR: a header always; while open (or unfolded) its checks and its review. */
function PrItem({ repo, worktree, cand, tick, target, onUnlinked }: { repo: Repo; worktree: string | null; cand: Candidate; tick: number; target: string | null; onUnlinked: () => void }) {
  const toast = useToast();
  // Polled while the PR is open: checks and reviews keep arriving after it is opened.
  const [live, setLive] = useState(true);
  const details = usePolled<PrDetails>(`repo-pr-details:${rootKey(repo.id, worktree)}:${cand.number}`, () => repoPrDetails(repo.id, worktree, cand.number), [], live ? DETAILS_POLL : 0);
  const [seenTick, setSeenTick] = useState(tick);
  if (seenTick !== tick) {
    setSeenTick(tick);
    void details.refresh();
  }
  const d = details.data;
  const state: PrState = d ? (d.state === "MERGED" ? "merged" : d.state === "CLOSED" ? "closed" : d.isDraft ? "draft" : "open") : (cand.current?.state ?? "open");
  const active = state === "open" || state === "draft";
  if (active !== live) setLive(active);
  const [unfolded, setUnfolded] = useState(false);
  const expanded = active || unfolded;
  const [sending, setSending] = useState<string | null>(null);

  const rows = (d?.statusCheckRollup ?? []).map(checkRow).sort((a, b) => ORDER[a.state] - ORDER[b.state]);
  const count = (s: CheckState) => rows.filter((r) => r.state === s).length;
  const summary = (["failed", "pending", "passed"] as const)
    .filter((s) => count(s) > 0)
    .map((s) => `${count(s)} ${s}`)
    .join(" · ");
  const now = Date.now();
  const reviews = (d?.reviews ?? []).filter((r) => r.body.trim() || r.state !== "COMMENTED");
  const comments = d?.comments ?? [];
  const inline = d?.reviewComments ?? [];
  const files = [...new Set(inline.map((c) => c.path))];
  const talk = reviews.length + comments.length + inline.length;
  const decision = d?.reviewDecision ? DECISION[d.reviewDecision] : undefined;
  const branch = cand.branch ?? d?.headRefName ?? null;

  const send = async (id: string, text: () => Promise<string>, label: string) => {
    if (!target) return;
    setSending(id);
    try {
      await sendToChat(target, await text());
      toast("Sent to the chat", label, "ok");
    } catch (e) {
      toast("Couldn't send to the chat", String(e), "danger");
    } finally {
      setSending(null);
    }
  };
  const sendFailure = (row: Row) =>
    send(
      `check:${row.name}`,
      async () => {
        const head = `The CI check "${row.name}" failed on PR #${cand.number} (${row.url}). Fix it.`;
        const runId = /\/actions\/runs\/(\d+)/.exec(row.url)?.[1];
        const log = runId ? await repoCheckLog(repo.id, worktree, runId) : null;
        return log ? `${head} Failed log (tail):\n\`\`\`\n${log}\n\`\`\`` : head;
      },
      row.name,
    );
  const sendReview = () =>
    send(
      "review",
      async () => {
        const parts = [`Address the review feedback on PR #${cand.number} (${cand.url}):`];
        for (const r of reviews) if (r.body.trim()) parts.push(`- ${r.author.login} (${r.state.toLowerCase()}): ${r.body.trim()}`);
        for (const c of comments) parts.push(`- ${c.author.login}: ${c.body.trim()}`);
        for (const c of inline) parts.push(`- ${c.path}${c.line ? `:${c.line}` : ""} (${c.author.login}): ${c.body.trim()}`);
        return parts.join("\n");
      },
      `Review of #${cand.number}`,
    );
  const unlink = () =>
    repoUnlinkPr(repo.id, worktree)
      .then(onUnlinked)
      .catch((e: unknown) => toast("Couldn't unlink the PR", String(e), "danger"));
  const noChat = target ? undefined : "Open a chat to send it";

  const head = (
    <>
      <span className={`pr-state pr-state-${state}`}>{STATE_LABEL[state]}</span>
      <span className="pr-head-text">
        <span className="pr-title">
          <span className="pr-num num">#{cand.number}</span> {d?.title ?? (details.error ? "Couldn't load this PR" : "Loading…")}
        </span>
        {branch && (
          <span className="pr-branch mono ellipsis" title={branch}>
            {branch}
          </span>
        )}
      </span>
    </>
  );

  return (
    <li className={`pr-item ${expanded ? "" : "pr-folded"}`}>
      <div className="pr-head">
        {active ? (
          <div className="pr-head-main">{head}</div>
        ) : (
          <button type="button" className="pr-head-main pr-head-btn" aria-expanded={unfolded} onClick={() => setUnfolded((v) => !v)}>
            {head}
          </button>
        )}
        <ExternalLink url={cand.url} className="icon-btn pr-open" label={`Open PR #${cand.number} on GitHub`}>
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
            <path d="M5 2.5H2.5v7h7V7M7 2h3v3M10 2 5.5 6.5" />
          </svg>
        </ExternalLink>
      </div>
      {details.error && (
        <p className="files-note pr-pad tr-error" role="alert">
          {details.error}
        </p>
      )}
      {expanded && d && (
        <div className="pr-body">
          {cand.current?.manual && (
            <p className="pr-pad pr-note">
              Linked by hand ·{" "}
              <button type="button" className="pr-text-btn" onClick={() => void unlink()}>
                Unlink
              </button>
            </p>
          )}
          {state === "open" && d.mergeable === "CONFLICTING" && <p className="pr-pad pr-note pr-bad">Has merge conflicts with the base branch.</p>}

          <Block title="Checks" meta={rows.length ? summary : "None reported"} tone={count("failed") ? "danger" : count("pending") ? "accent" : rows.length ? "ok" : null}>
            {rows.length > 0 && (
              <ul className="pr-checks" aria-label={`Checks of #${cand.number}`}>
                {rows.map((r, i) => (
                  <li key={`${r.name}:${i}`} className={`pr-check pr-check-${r.state}`}>
                    <CheckIcon state={r.state} />
                    <ExternalLink url={r.url} className="ellipsis pr-check-name" label={`Open check ${r.name}`}>
                      {r.name}
                    </ExternalLink>
                    {r.state === "failed" && (
                      <button type="button" className="pr-act pr-act-danger" disabled={!target || sending !== null} title={noChat ?? "Send the failure and its log to the chat to fix it"} onClick={() => void sendFailure(r)}>
                        {sending === `check:${r.name}` ? "Sending…" : "Fix in chat"}
                      </button>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </Block>

          <Block
            title="Review"
            meta={decision ? <span className={`pr-decision tone-${decision.tone}`}>{decision.label}</span> : talk ? `${talk} ${talk === 1 ? "comment" : "comments"}` : "No comments yet"}
            tone={null}
            action={
              talk > 0 && (
                <button type="button" className="pr-act" disabled={!target || sending !== null} title={noChat ?? "Send every review comment to the chat to address"} onClick={() => void sendReview()}>
                  {sending === "review" ? "Sending…" : "Send to chat"}
                </button>
              )
            }
          >
            {talk > 0 && (
              <div className="pr-thread">
                {[...reviews.map((r) => ({ id: `r:${r.author.login}:${r.submittedAt}`, who: r.author.login, at: r.submittedAt, label: r.state, body: r.body })), ...comments.map((c) => ({ id: c.url, who: c.author.login, at: c.createdAt, label: "", body: c.body }))]
                  .sort((a, b) => Date.parse(a.at) - Date.parse(b.at))
                  .map((c) => (
                    <div key={c.id} className="pr-comment">
                      <span className="pr-comment-meta">
                        <span className="pr-comment-who">{c.who}</span>
                        {c.label && <span className="pr-comment-kind">{c.label.toLowerCase().replace(/_/g, " ")}</span>}
                        {!Number.isNaN(Date.parse(c.at)) && <span>{shortAgo(Date.parse(c.at), now)}</span>}
                      </span>
                      {c.body.trim() && <SafeMarkdown text={c.body} />}
                    </div>
                  ))}
                {files.map((path) => (
                  <div key={path} className="pr-file">
                    <span className="pr-file-path mono ellipsis" title={path}>
                      {path}
                    </span>
                    {inline
                      .filter((c) => c.path === path)
                      .sort((a, b) => (a.line ?? 0) - (b.line ?? 0))
                      .map((c) => (
                        <div key={c.url} className="pr-inline">
                          <ExternalLink url={c.url} className="pr-line mono num" label={`Open the comment on line ${c.line ?? "?"}`}>
                            {c.line != null ? `L${c.line}` : "—"}
                          </ExternalLink>
                          <div className="pr-inline-body">
                            <span className="pr-comment-meta">
                              <span className="pr-comment-who">{c.author.login}</span>
                              {!Number.isNaN(Date.parse(c.createdAt)) && <span>{shortAgo(Date.parse(c.createdAt), now)}</span>}
                            </span>
                            <SafeMarkdown text={c.body} />
                          </div>
                        </div>
                      ))}
                  </div>
                ))}
              </div>
            )}
          </Block>

          {state === "open" && <MergeBar repo={repo} worktree={worktree} number={cand.number} title={d.title} conflicts={d.mergeable === "CONFLICTING"} failed={count("failed")} pending={count("pending")} onMerged={() => {
            void details.refresh();
            onUnlinked();
          }} />}
        </div>
      )}
    </li>
  );
}

const MERGE_LABEL: Record<MergeMethod, string> = { squash: "Squash and merge", merge: "Create a merge commit", rebase: "Rebase and merge" };

/** Merges an open PR on GitHub, squash by default; asks first, and says what's still red. */
function MergeBar({ repo, worktree, number, title, conflicts, failed, pending, onMerged }: { repo: Repo; worktree: string | null; number: number; title: string; conflicts: boolean; failed: number; pending: number; onMerged: () => void }) {
  const ask = useConfirm();
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const warn = [failed > 0 && `${failed} ${failed === 1 ? "check failed" : "checks failed"}`, pending > 0 && `${pending} still running`].filter(Boolean).join(", ");
  const merge = async (method: MergeMethod) => {
    const ok = await ask({
      title: `${MERGE_LABEL[method]} #${number}?`,
      body: (
        <>
          “{title}” is merged on GitHub. The branch is kept.
          {warn && <> Careful: {warn}.</>}
        </>
      ),
      confirmLabel: MERGE_LABEL[method],
      destructive: !!warn,
    });
    if (!ok) return;
    setBusy(true);
    try {
      await repoMergePr(repo.id, worktree, number, method);
      toast("Merged", `#${number} ${title}`, "ok");
      onMerged();
    } catch (e) {
      toast("Couldn't merge the PR", String(e), "danger");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div className="pr-merge">
      {conflicts ? (
        <span className="pr-merge-note pr-bad">Resolve the conflicts to merge.</span>
      ) : (
        warn && <span className="pr-merge-note pr-bad">{warn}</span>
      )}
      <DropdownMenu.Root>
        <div className="pr-merge-btns">
          <button type="button" className="btn btn-sm btn-primary" disabled={busy || conflicts} onClick={() => void merge("squash")}>
            {busy ? "Merging…" : "Squash and merge"}
          </button>
          <DropdownMenu.Trigger className="btn btn-sm btn-primary pr-merge-more" disabled={busy || conflicts} aria-label="Other ways to merge">
            <svg width="9" height="9" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
              <path d="M2.5 4l2.5 2.5L7.5 4" />
            </svg>
          </DropdownMenu.Trigger>
        </div>
        <DropdownMenu.Portal>
          <DropdownMenu.Content className="menu tab-menu" align="end" sideOffset={4} collisionPadding={8}>
            {(["merge", "rebase"] as const).map((m) => (
              <DropdownMenu.Item key={m} className="menu-item" onSelect={() => void merge(m)}>
                {MERGE_LABEL[m]}
              </DropdownMenu.Item>
            ))}
          </DropdownMenu.Content>
        </DropdownMenu.Portal>
      </DropdownMenu.Root>
    </div>
  );
}

function Block({ title, meta, tone, action, children }: { title: string; meta: ReactNode; tone: string | null; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="pr-block">
      <div className="pr-block-head">
        <span className="pr-block-title">{title}</span>
        <span className={`pr-block-meta ${tone ? `tone-${tone} pr-toned` : ""}`}>{meta}</span>
        {action}
      </div>
      {children}
    </section>
  );
}

function CheckIcon({ state }: { state: CheckState }) {
  const path = state === "failed" ? "M4 4l4 4M8 4l-4 4" : state === "passed" ? "M3.5 6.2l1.8 1.8 3.4-3.6" : null;
  return (
    <span className={`pr-check-icon ${state === "pending" ? "pulse" : ""}`} role="img" aria-label={state}>
      <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
        {path ? <path d={path} /> : <circle cx="6" cy="6" r="2" fill="currentColor" stroke="none" />}
      </svg>
    </span>
  );
}

/** Links a PR by URL or number; without any PR it also offers to open one. */
function LinkForm({ repo, worktree, chatId, onLinked, onAskChatForPr }: { repo: Repo; worktree: string | null; chatId: string | null; onLinked: () => void; onAskChatForPr: (() => void) | null }) {
  const toast = useToast();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);
  const link = async () => {
    setBusy(true);
    try {
      await repoLinkPr(repo.id, worktree, value.trim(), chatId);
      setValue("");
      onLinked();
    } catch (e) {
      toast("Couldn't link the PR", String(e), "danger");
    } finally {
      setBusy(false);
    }
  };
  return (
    <form
      className="git-pr-form pr-link-form"
      onSubmit={(e) => {
        e.preventDefault();
        if (value.trim()) void link();
      }}
    >
      <label className="git-label" htmlFor={`pr-link-${repo.id}`}>
        Link a pull request
      </label>
      <input id={`pr-link-${repo.id}`} className="input git-input" placeholder="PR URL or number" value={value} onChange={(e) => setValue(e.target.value)} />
      <div className="git-actions">
        <button type="submit" className="btn btn-sm btn-primary" disabled={!value.trim() || busy}>
          {busy ? "Linking…" : "Link PR"}
        </button>
        {onAskChatForPr && (
          <button type="button" className="btn btn-sm" disabled={!worktree} title={worktree ? undefined : "Open a PR from a worktree branch"} onClick={onAskChatForPr}>
            Open PR
          </button>
        )}
      </div>
    </form>
  );
}
