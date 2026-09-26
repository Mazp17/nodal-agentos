import { useState, type ReactNode } from "react";
import { createTask, launchTask, respondChatPermission, type PermissionRequest } from "../../domain/api";
import { invalidate } from "../../domain/hooks/store";
import { taskKey, type Repo } from "../../domain/types";
import { readJsonPref, writePref } from "../../shell/storage";
import { BrandMark } from "../../ui/BrandMark";
import { SafeMarkdown } from "../../ui/Markdown";
import { useToast } from "../../ui/Toasts";
import { DiffLines } from "../runs/DiffLines";
import { PriorityBars } from "../tasks/bits";
import { PRIORITY_LABEL, STATUS_META } from "../tasks/status";
import { readProposal, resultMeta, toolLabel, type Proposal, type ToolUse, type Turn } from "./model";
import type { ChatStream } from "./stream";

// ---------- Tool block ----------

/** Diffs up to this many lines start open; longer ones wait for a click. */
const DIFF_OPEN_MAX = 20;

function ToolRow({ item, first, denied, asking }: { item: ToolUse; first: boolean; denied: string | undefined; asking: boolean }) {
  const [toggled, setOpen] = useState<boolean | null>(null);
  const r = item.result;
  const patch = !denied && r?.patch ? r.patch : null;
  const lines = patch ? patch.file.hunks.reduce((n, h) => n + h.lines.length, 0) : 0;
  const open = toggled ?? (patch != null && lines <= DIFF_OPEN_MAX);
  const tone = denied || r?.isError ? "danger" : asking ? "warn" : r ? "ok" : "accent";
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
  ) : (
    "Running…"
  );
  const expandable = item.input != null || r != null;
  return (
    <div className={`chat-tool ${first ? "" : "chat-tool-sep"}`}>
      <button
        type="button"
        className="chat-tool-head"
        aria-expanded={expandable ? open : undefined}
        disabled={!expandable}
        onClick={() => setOpen(!open)}
      >
        <span className={`dot dot-sm tone-${tone} ${!r && !denied && !asking ? "pulse" : ""}`} aria-hidden />
        <span className="chat-tool-name">{toolLabel(item.name)}</span>
        <span className="chat-tool-arg ellipsis">{item.summary ?? ""}</span>
        <span className={`chat-tool-meta ${tone === "danger" ? "chat-tool-meta-err" : ""}`}>{meta}</span>
      </button>
      {open && patch && (
        <div className="chat-tool-diff">
          <DiffLines file={patch.file} />
          {patch.truncated && <p className="diff-note">Showing the first {lines} lines.</p>}
        </div>
      )}
      {open && !patch && (
        <div className="chat-tool-body">
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

export function ToolBlock({ items, stream }: { items: ToolUse[]; stream: ChatStream }) {
  const asking = new Set(stream.pending.map((p) => p.toolUseId).filter(Boolean));
  return (
    <div className="chat-tools" role="group" aria-label="Tool calls">
      {items.map((it, i) => (
        <ToolRow
          key={it.id ?? i}
          item={it}
          first={i === 0}
          denied={it.id ? stream.denied[it.id] : undefined}
          asking={!!it.id && asking.has(it.id)}
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

  const create = async (run: boolean) => {
    setBusy(true);
    try {
      const task = await createTask(t);
      const key = taskKey(projectKey, task.number);
      if (item.id) rememberCreated(item.id, task.id, key);
      setCreated({ taskId: task.id, key });
      void invalidate("tasks");
      if (run) {
        try {
          const r = await launchTask(task.id);
          toast(r.status === "queued" ? "Queued" : "Launching", `${key} · ${task.title}`, "accent");
          void invalidate("runs", "tasks");
        } catch (e) {
          toast(`${key} created, but couldn't launch`, String(e), "danger");
        }
      } else {
        toast("Task created", `${key} · ${task.title}`, "ok");
      }
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
            <button type="button" className="btn btn-sm btn-ghost" onClick={() => onOpenTask(created.taskId)}>
              Open task
            </button>
          </>
        ) : (
          <>
            <span className="chat-proposal-hint">Lands in {status} · local task</span>
            <button type="button" className="btn btn-sm" disabled={busy} onClick={() => void create(false)}>
              Create task
            </button>
            <button type="button" className="btn btn-sm btn-primary" disabled={busy} onClick={() => void create(true)}>
              Create &amp; run
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

export function Conversation({ chatId, turns, stream, projectId, projectKey, repos, onOpenTask }: ConversationProps) {
  const busy = stream.state === "busy" || stream.outbox.length > 0;
  const lastTool = [...stream.items].reverse().find((i): i is ToolUse => i.kind === "toolUse");
  const working =
    stream.pending.length > 0
      ? "Waiting for your approval"
      : stream.thinking
        ? "Thinking…"
        : lastTool && !lastTool.result && !stream.streaming
          ? `Running ${toolLabel(lastTool.name)}…`
          : "Working…";
  const lastIsAi = turns[turns.length - 1]?.kind === "ai";

  return (
    <div className="chat-thread">
      {stream.omitted > 0 && (
        <div className="chat-omitted">
          {stream.omitted} earlier item{stream.omitted === 1 ? "" : "s"} not shown. `claude --resume` in the repo shows the whole
          session.
        </div>
      )}
      {turns.map((turn, i) =>
        turn.kind === "user" ? (
          <div key={i} className="chat-user">
            <div className={`chat-bubble ${turn.pending ? "chat-bubble-pending" : ""}`}>{turn.text}</div>
          </div>
        ) : (
          <AiTurn key={i}>
            {turn.parts.map((p, j) => {
              switch (p.kind) {
                case "text":
                  return <SafeMarkdown key={j} className="chat-text" text={p.text} breaks />;
                case "thinking":
                  return (
                    <details key={j} className="tr-thinking">
                      <summary>Thinking</summary>
                      <div>{p.text}</div>
                    </details>
                  );
                case "tools":
                  return <ToolBlock key={j} items={p.items} stream={stream} />;
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
            {i === turns.length - 1 && stream.streaming && <div className="chat-text chat-streaming">{stream.streaming}</div>}
          </AiTurn>
        ),
      )}
      {!lastIsAi && stream.streaming && (
        <AiTurn>
          <div className="chat-text chat-streaming">{stream.streaming}</div>
        </AiTurn>
      )}
      {stream.pending.map((req) => (
        <ApprovalCard key={req.requestId} chatId={chatId} req={req} />
      ))}
      {busy && (
        <div className="chat-working" role="status">
          <span className="chat-avatar chat-avatar-pulse">
            <BrandMark size={22} />
          </span>
          {working}
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
