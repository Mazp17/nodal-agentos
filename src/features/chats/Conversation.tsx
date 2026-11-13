import { useState, type ReactNode } from "react";
import { createTask, respondChatPermission, type PermissionRequest } from "../../domain/api";
import { invalidate } from "../../domain/hooks/store";
import { taskKey, type Repo } from "../../domain/types";
import { readJsonPref, writePref } from "../../shell/storage";
import { BrandMark } from "../../ui/BrandMark";
import { Marker, MarkerContent, MarkerIcon } from "@/ui/marker";
import { SafeMarkdown } from "../../ui/Markdown";
import { useToast } from "../../ui/Toasts";
import { DiffLines } from "./DiffLines";
import { useOpenTaskChat } from "../tasks";
import { PriorityBars } from "../tasks/bits";
import { PRIORITY_LABEL, STATUS_META } from "../tasks/status";
import {
  readProposal,
  resultMeta,
  shortArg,
  toolLabel,
  toolVerb,
  turnChanges,
  type Proposal,
  type ToolUse,
  type Turn,
} from "./model";
import type { ChatStream } from "./stream";

// ---------- Tool block ----------

/** Diffs up to this many lines start open; longer ones wait for a click. */
const DIFF_OPEN_MAX = 20;

type ToolState = "running" | "asking" | "ok" | "error" | "stopped";

/** The badge before a tool row: a check once done, a pulsing dot while it runs. */
function ToolIcon({ state }: { state: ToolState }) {
  return (
    // A node on the answer's trail: the state is its ring and glyph.
    <MarkerIcon className={`chat-mk-icon chat-mk-${state}`}>
      {state !== "running" && (
        <svg width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" aria-hidden focusable="false">
          {state === "ok" && <path d="M1.6 4.2l1.6 1.6 3.2-3.6" />}
          {state === "error" && <path d="M2 2l4 4M6 2L2 6" />}
          {state === "asking" && <path d="M4 1.6v2.6M4 6.2v.1" />}
          {state === "stopped" && <path d="M2 4h4" />}
        </svg>
      )}
    </MarkerIcon>
  );
}

function ToolRow({
  item,
  repos,
  denied,
  asking,
  live,
  onOpenDiff,
}: {
  item: ToolUse;
  repos: Repo[];
  denied: string | undefined;
  asking: boolean;
  live: boolean;
  onOpenDiff?: (path: string) => void;
}) {
  const [toggled, setOpen] = useState<boolean | null>(null);
  const r = item.result;
  const patch = !denied && r?.patch ? r.patch : null;
  const lines = patch ? patch.file.hunks.reduce((n, h) => n + h.lines.length, 0) : 0;
  const open = toggled ?? (patch != null && lines <= DIFF_OPEN_MAX);
  // A call without a result after the turn ended was interrupted.
  const state: ToolState = denied || r?.isError ? "error" : asking ? "asking" : r ? "ok" : live ? "running" : "stopped";
  const meta: ReactNode = denied ? (
    "Denied"
  ) : asking ? (
    "Needs approval"
  ) : patch ? (
    <>
      <span className="diff-add">+{patch.file.additions}</span> <span className="diff-del">−{patch.file.deletions}</span>
    </>
  ) : r ? (
    r.isError ? "Error" : resultMeta(r.text)
  ) : state === "stopped" ? (
    "No result"
  ) : null;
  const expandable = item.input != null || r != null || denied != null;
  const arg = shortArg(item.summary, repos);
  const label = (
    <>
      <ToolIcon state={state} />
      <MarkerContent className={`chat-mk-text ${state === "running" ? "shimmer" : ""}`}>
        <span className="chat-mk-verb">{toolVerb(item.name, state === "running")}</span>
        {arg && <span className="chat-mk-arg">{arg}</span>}
        {meta && <span className={`chat-mk-meta ${state === "error" ? "chat-mk-meta-err" : ""}`}>{meta}</span>}
        {expandable && (
          <span className="chat-mk-chev" aria-hidden>
            {open ? "▾" : "▸"}
          </span>
        )}
      </MarkerContent>
    </>
  );
  return (
    <div className="chat-tool">
      <div className="chat-tool-line">
        {expandable ? (
          <Marker asChild className="chat-mk chat-mk-btn">
            <button type="button" aria-expanded={open} onClick={() => setOpen(!open)}>
              {label}
            </button>
          </Marker>
        ) : (
          <Marker className="chat-mk">
            {label}
          </Marker>
        )}
        {patch && onOpenDiff && (
          <button type="button" className="chat-tool-open" title="Open in git diff" onClick={() => onOpenDiff(patch.file.path)}>
            Open diff
          </button>
        )}
      </div>
      {open && patch && (
        <div className="chat-tool-panel">
          <div className="chat-tool-panel-head">
            {(() => {
              const rel = shortArg(patch.file.path, repos) || patch.file.path;
              const slash = rel.lastIndexOf("/");
              return (
                <span className="chat-tool-panel-path ellipsis" title={patch.file.path}>
                  {slash >= 0 && <span className="chat-tool-panel-dir">{rel.slice(0, slash + 1)}</span>}
                  <span className="chat-tool-panel-name">{rel.slice(slash + 1)}</span>
                </span>
              );
            })()}
            <span className="chat-tool-panel-stat num">
              <span className="diff-add">+{patch.file.additions}</span>
              <span className="diff-del">−{patch.file.deletions}</span>
            </span>
            {onOpenDiff && (
              <button type="button" className="chat-tool-panel-open" onClick={() => onOpenDiff(patch.file.path)}>
                Open diff
              </button>
            )}
          </div>
          <DiffLines file={patch.file} compact />
          {patch.truncated && <p className="diff-note">Showing the first {lines} lines.</p>}
        </div>
      )}
      {open && !patch && (
        <div className="chat-tool-panel chat-tool-body">
          {item.input && <pre className="tr-pre">{item.input}</pre>}
          {denied && <pre className="tr-pre">{denied}</pre>}
          {r && (
            <pre className="tr-pre">
              {r.text || "(empty)"}
              {r.truncated && <span className="tr-dim"> [truncated]</span>}
            </pre>
          )}
        </div>
      )}
    </div>
  );
}

export function ToolBlock({
  items,
  stream,
  repos,
  live,
  onOpenDiff,
}: {
  items: ToolUse[];
  stream: ChatStream;
  repos: Repo[];
  /** The answer is still being written: calls without a result are running. */
  live: boolean;
  /** Opens the file's uncommitted changes (the edited path, as the tool reported it). */
  onOpenDiff?: (path: string) => void;
}) {
  const asking = new Set(stream.pending.map((p) => p.toolUseId).filter(Boolean));
  return (
    <div className="chat-tools" role="group" aria-label="Tool calls">
      {items.map((it, i) => (
        <ToolRow
          key={it.id ?? i}
          item={it}
          repos={repos}
          denied={it.id ? stream.denied[it.id] : undefined}
          asking={!!it.id && asking.has(it.id)}
          live={live}
          onOpenDiff={onOpenDiff}
        />
      ))}
    </div>
  );
}

// ---------- Approval card ----------

export function ApprovalCard({ chatId, req }: { chatId: string; req: PermissionRequest }) {
  const toast = useToast();
  const [note, setNote] = useState("");
  const [busy, setBusy] = useState(false);
  const [showInput, setShowInput] = useState(false);
  const input = req.input == null ? null : JSON.stringify(req.input, null, 2);

  const answer = (allow: boolean) => {
    setBusy(true);
    respondChatPermission(chatId, req.requestId, allow, allow ? null : note.trim() || null).then(
      // The card goes away with `permissionResolved`.
      () => {},
      (e: unknown) => {
        setBusy(false);
        toast("Couldn't answer the prompt", String(e), "danger");
      },
    );
  };

  return (
    <div className="chat-approval" role="group" aria-label={`Permission request: ${toolLabel(req.toolName)}`}>
      <div className="chat-approval-head">
        <span className="dot dot-sm tone-warn" aria-hidden />
        <span>
          Claude wants to use <strong>{toolLabel(req.toolName)}</strong>
        </span>
      </div>
      {req.description && <div className="chat-approval-desc">{req.description}</div>}
      {req.summary && <code className="chat-approval-arg">{req.summary}</code>}
      {input && (
        <button type="button" className="chat-link-btn" aria-expanded={showInput} onClick={() => setShowInput(!showInput)}>
          {showInput ? "Hide details" : "Show details"}
        </button>
      )}
      {showInput && input && <pre className="tr-pre chat-approval-input">{input}</pre>}
      <div className="chat-approval-actions">
        <input
          className="input chat-approval-note"
          value={note}
          placeholder="Tell Claude why, if you deny (optional)"
          aria-label="Reason for denying"
          disabled={busy}
          onChange={(e) => setNote(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.nativeEvent.isComposing) {
              e.preventDefault();
              answer(false);
            }
          }}
        />
        <button type="button" className="btn btn-sm" disabled={busy} onClick={() => answer(false)}>
          Deny
        </button>
        <button type="button" className="btn btn-sm btn-amber" disabled={busy} onClick={() => answer(true)}>
          Allow
        </button>
      </div>
    </div>
  );
}

// ---------- Proposed task card ----------

/** Tasks created from a card, by tool use id: the card keeps showing "created". */
const CREATED_PREF = "chatProposals";
const CREATED_MAX = 300;
type Created = Record<string, { taskId: string; key: string }>;
const isCreated = (v: unknown): v is Created => !!v && typeof v === "object" && !Array.isArray(v);

function rememberCreated(toolUseId: string, taskId: string, key: string) {
  const all = readJsonPref<Created>(CREATED_PREF, {}, isCreated);
  const entries = Object.entries({ ...all, [toolUseId]: { taskId, key } }).slice(-CREATED_MAX);
  writePref(CREATED_PREF, JSON.stringify(Object.fromEntries(entries)));
}

function ProposalCard({
  item,
  projectId,
  projectKey,
  repos,
  onOpenTask,
}: {
  item: ToolUse;
  projectId: string;
  projectKey: string;
  repos: Repo[];
  onOpenTask: (taskId: string) => void;
}) {
  const toast = useToast();
  const [created, setCreated] = useState(() =>
    item.id ? (readJsonPref<Created>(CREATED_PREF, {}, isCreated)[item.id] ?? null) : null,
  );
  const [busy, setBusy] = useState(false);
  const openTaskChat = useOpenTaskChat();
  const st = readProposal(item, projectId, repos);

  if (st.status === "pending") {
    return (
      <div className="chat-proposal chat-proposal-note">
        <span className="dot dot-sm pulse tone-accent" aria-hidden /> Preparing a task…
      </div>
    );
  }
  if (st.status === "error") {
    return <div className="chat-proposal chat-proposal-note chat-proposal-err">Couldn't propose the task: {st.message}</div>;
  }
  if (st.status === "unreadable") {
    return (
      <div className="chat-proposal chat-proposal-note">
        Claude proposed a task{st.title ? ` (“${st.title}”)` : ""}, but it's too long to show here. Ask Claude to create it
        with a shorter plan, or create it from the board.
      </div>
    );
  }

  const p: Proposal = st.proposal;
  const t = p.newTask;
  const priority = t.priority ?? "none";
  const status = STATUS_META[t.status ?? "todo"].label;

  const create = async (chat: boolean) => {
    setBusy(true);
    try {
      const task = await createTask(t);
      const key = taskKey(projectKey, task.number);
      if (item.id) rememberCreated(item.id, task.id, key);
      setCreated({ taskId: task.id, key });
      void invalidate("tasks");
      if (chat) await openTaskChat(task.id);
      else toast("Task created", `${key} · ${task.title}`, "ok");
    } catch (e) {
      toast("Couldn't create the task", String(e), "danger");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="chat-proposal" role="group" aria-label={`Proposed task: ${t.title}`}>
      <div className="chat-proposal-body">
        <div className="chat-proposal-meta">
          <span className="chat-proposal-kicker">Proposed task</span>
          {priority !== "none" && (
            <span className="chat-proposal-prio">
              <PriorityBars priority={priority} />
              {PRIORITY_LABEL[priority]}
            </span>
          )}
          {p.repoName && <span className="mono">{p.repoName}</span>}
        </div>
        <div className="chat-proposal-title">{t.title}</div>
        {t.plan.kind === "text" && <SafeMarkdown className="chat-proposal-plan" text={t.plan.text} />}
        {(t.acceptance?.length ?? 0) > 0 && (
          <ul className="chat-proposal-crit" aria-label="Acceptance criteria">
            {t.acceptance?.map((a, i) => <li key={i}>{a}</li>)}
          </ul>
        )}
      </div>
      <div className="chat-proposal-foot">
        {created ? (
          <>
            <span className="chat-proposal-done">
              ✓ <span className="mono">{created.key}</span> created
            </span>
            <button type="button" className="chat-proposal-open" onClick={() => onOpenTask(created.taskId)}>
              Open task
            </button>
          </>
        ) : (
          <>
            <span className="chat-proposal-hint">Lands in {status} · local task</span>
            <button type="button" className="btn" disabled={busy} onClick={() => void create(false)}>
              Create task
            </button>
            <button type="button" className="btn btn-primary" disabled={busy} onClick={() => void create(true)}>
              Create &amp; open chat
            </button>
          </>
        )}
      </div>
    </div>
  );
}

// ---------- Conversation ----------

export interface ConversationProps {
  chatId: string;
  turns: Turn[];
  stream: ChatStream;
  projectId: string;
  projectKey: string;
  repos: Repo[];
  onOpenTask: (taskId: string) => void;
  onOpenDiff?: (path: string) => void;
}

function AiTurn({ children }: { children: ReactNode }) {
  return (
    <div className="chat-ai">
      <span className="chat-avatar">
        <BrandMark size={22} />
      </span>
      <div className="chat-ai-body">{children}</div>
    </div>
  );
}

/** The files an answer changed, under its tool calls. */
function ChangesMarker({ changes }: { changes: NonNullable<ReturnType<typeof turnChanges>> }) {
  return (
    <Marker variant="border" className="chat-mk chat-mk-changes">
      <MarkerIcon className="chat-mk-icon chat-mk-changed">
        <svg width="8" height="8" viewBox="0 0 8 8" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" aria-hidden focusable="false">
          <path d="M4 1.5v5M1.5 4h5" />
        </svg>
      </MarkerIcon>
      <MarkerContent className="chat-mk-text">
        {changes.files} {changes.files === 1 ? "file" : "files"} changed
        <span className="chat-mk-meta">
          <span className="diff-add">+{changes.additions}</span> <span className="diff-del">−{changes.deletions}</span>
        </span>
      </MarkerContent>
    </Marker>
  );
}

export function Conversation({ chatId, turns, stream, projectId, projectKey, repos, onOpenTask, onOpenDiff }: ConversationProps) {
  const busy = stream.state === "busy" || stream.outbox.length > 0;
  const last = turns[turns.length - 1];
  const lastPart = last?.kind === "ai" ? last.parts[last.parts.length - 1] : undefined;
  const waiting = new Set(stream.pending.map((p) => p.toolUseId).filter(Boolean));
  // Calls of the answer being written that still run (not waiting for approval nor denied).
  const running =
    stream.state === "busy" && !stream.streaming && !stream.thinking && lastPart?.kind === "tools"
      ? lastPart.items.filter((it) => !it.result && !(it.id && (waiting.has(it.id) || stream.denied[it.id])))
      : [];
  // A running tool call shows itself (its row shimmers); this line covers the rest of the turn.
  const working =
    stream.pending.length > 0
      ? "Waiting for your approval"
      : stream.thinking
        ? "Thinking…"
        : stream.streaming
          ? "Writing…"
          : "Working…";
  const running0 = running[running.length - 1];
  const announced = !busy
    ? ""
    : running0
      ? `${toolVerb(running0.name, true)} ${shortArg(running0.summary, repos)}`.trim()
      : working;
  const lastIsAi = turns[turns.length - 1]?.kind === "ai";

  return (
    <div className="chat-thread">
      {stream.omitted > 0 && (
        <Marker variant="separator" className="chat-mk chat-mk-omitted">
          <MarkerContent>
            {stream.omitted} earlier item{stream.omitted === 1 ? "" : "s"} not shown · `claude --resume` in the repo shows the
            whole session
          </MarkerContent>
        </Marker>
      )}
      {turns.map((turn, i) => {
        if (turn.kind === "user") {
          return (
            <div key={i} className="chat-user">
              <div className={`chat-bubble ${turn.pending ? "chat-bubble-pending" : ""}`}>{turn.text}</div>
            </div>
          );
        }
        const current = i === turns.length - 1;
        const changes = current && busy ? null : turnChanges(turn.parts);
        return (
          <AiTurn key={i}>
            {turn.parts.map((p, j) => {
              switch (p.kind) {
                case "text":
                  return <SafeMarkdown key={j} className="chat-text" text={p.text} breaks />;
                case "thinking":
                  return (
                    <details key={j} className="chat-thought">
                      <Marker asChild className="chat-mk chat-mk-btn">
                        <summary>
                          <MarkerIcon className="chat-mk-icon chat-mk-stopped">…</MarkerIcon>
                          <MarkerContent className="chat-mk-text">
                            <span className="chat-mk-verb">Thought</span>
                          </MarkerContent>
                        </summary>
                      </Marker>
                      <div className="chat-thought-text">{p.text}</div>
                    </details>
                  );
                case "tools":
                  return <ToolBlock key={j} items={p.items} stream={stream} repos={repos} live={current && stream.state === "busy"} onOpenDiff={onOpenDiff} />;
                case "proposal":
                  return (
                    <ProposalCard
                      key={p.item.id ?? j}
                      item={p.item}
                      projectId={projectId}
                      projectKey={projectKey}
                      repos={repos}
                      onOpenTask={onOpenTask}
                    />
                  );
              }
            })}
            {changes && <ChangesMarker changes={changes} />}
            {current && stream.streaming && <div className="chat-text chat-streaming">{stream.streaming}</div>}
          </AiTurn>
        );
      })}
      {!lastIsAi && stream.streaming && (
        <AiTurn>
          <div className="chat-text chat-streaming">{stream.streaming}</div>
        </AiTurn>
      )}
      {stream.pending.map((req) => (
        <ApprovalCard key={req.requestId} chatId={chatId} req={req} />
      ))}
      <span className="sr-only" role="status">
        {announced}
      </span>
      {busy && running.length === 0 && (
        <div className="chat-working" aria-hidden>
          <span className="chat-avatar chat-avatar-pulse">
            <BrandMark size={22} />
          </span>
          <Marker className="chat-mk chat-mk-working">
            <MarkerContent className={stream.pending.length > 0 ? "" : "shimmer"}>{working}</MarkerContent>
          </Marker>
        </div>
      )}
      {stream.notice && !busy && (
        <div className="chat-notice" role="status">
          {stream.notice}
        </div>
      )}
    </div>
  );
}
