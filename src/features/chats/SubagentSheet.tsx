import { useCallback, useEffect, useState, useSyncExternalStore } from "react";
import { getChatSubagentTranscript } from "../../domain/api";
import { useChatSubagents } from "../../domain/hooks/chats";
import { formatDuration } from "../../lib/format";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { agentState, agentStats, toolVerb, type ToolUse } from "./model";
import type { ChatStream } from "./stream";
import type { Transcript, TranscriptItem } from "./types";

// ---------- Which subagent's sheet is open ----------

/** The subagent whose sheet is open: set by a card or an Agents row, read by the chat's view. */
export interface SubagentTarget {
  chatId: string;
  /** The `Agent` call that launched it, when known. */
  toolUseId: string | null;
  agentId: string | null;
  title: string;
}

let target: SubagentTarget | null = null;
const listeners = new Set<() => void>();
const emit = (t: SubagentTarget | null) => {
  target = t;
  listeners.forEach((l) => l());
};

export const openSubagent = (t: SubagentTarget) => emit(t);
export const closeSubagent = () => emit(null);

/** The open sheet's subagent, if it belongs to `chatId`. */
export function useSubagentTarget(chatId: string | null): SubagentTarget | null {
  const subscribe = useCallback((l: () => void) => {
    listeners.add(l);
    return () => listeners.delete(l);
  }, []);
  const t = useSyncExternalStore(subscribe, () => target);
  return t && t.chatId === chatId ? t : null;
}

// ---------- Sheet ----------

const firstLine = (s: string) => {
  const line = s.split("\n").find((l) => l.trim()) ?? "";
  return line.length > 200 ? `${line.slice(0, 200)}…` : line;
};

function ToolCard({ item }: { item: ToolUse }) {
  const [open, setOpen] = useState(false);
  const r = item.result;
  const expandable = item.input != null || (r != null && r.text.includes("\n"));
  return (
    <div className={`tr-tool ${r?.isError ? "tr-tool-err" : ""}`}>
      <button type="button" className="tr-tool-head" aria-expanded={expandable ? open : undefined} disabled={!expandable} onClick={() => setOpen(!open)}>
        <span className="tr-tool-name">{toolVerb(item.name, r == null)}</span>
        <span className="tr-tool-arg ellipsis">{item.summary ?? ""}</span>
        {expandable && (
          <span className="tr-tool-chev" aria-hidden>
            {open ? "▾" : "▸"}
          </span>
        )}
      </button>
      {open && item.input && <pre className="tr-pre">{item.input}</pre>}
      <div className="tr-tool-res">
        {r == null ? (
          <span className="tr-dim">→ waiting for result…</span>
        ) : open ? (
          <pre className="tr-pre tr-pre-res">
            {r.text || "(empty)"}
            {r.truncated && <span className="tr-dim"> [truncated]</span>}
          </pre>
        ) : (
          <>
            → {r.isError ? "Error: " : ""}
            {firstLine(r.text) || "(empty)"}
          </>
        )}
      </div>
    </div>
  );
}

function Item({ item }: { item: TranscriptItem }) {
  switch (item.kind) {
    case "text":
      return <div className="tr-msg">{item.text}</div>;
    case "user":
      return (
        <div className="tr-user">
          <span className="tr-user-label">Prompt</span>
          {item.text}
        </div>
      );
    case "thinking":
      return (
        <details className="tr-thinking">
          <summary>Thinking</summary>
          <div>{item.text}</div>
        </details>
      );
    case "toolUse":
      return <ToolCard item={item} />;
  }
}

const STATE_LABEL = { running: "Running", ok: "Done", error: "Failed", stopped: "Stopped" } as const;
const STATE_TONE = { running: "accent", ok: "ok", error: "danger", stopped: "muted" } as const;

/** A subagent's conversation in a right-side sheet: live while it runs in this session, else read from its file. */
export function SubagentSheet({ target: t, stream, live, onClose }: { target: SubagentTarget; stream: ChatStream; live: boolean; onClose: () => void }) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const call = stream.items.find((i): i is ToolUse => i.kind === "toolUse" && !!t.toolUseId && i.id === t.toolUseId) ?? null;
  const refs = useChatSubagents(t.chatId).data;
  const meta = refs?.find((r) => r.agentId === t.agentId || (!!t.toolUseId && r.toolUseId === t.toolUseId));
  const agentId = t.agentId ?? call?.result?.agent?.agentId ?? meta?.agentId ?? null;
  const liveItems = t.toolUseId ? stream.subagents[t.toolUseId] : undefined;
  // A busy chat alone doesn't make an old unfinished call live: it must have streamed in this session.
  const state = call ? agentState(call, live && !!liveItems) : meta?.stoppedByUser ? "stopped" : "ok";
  // Items streamed now beat the file while the agent runs; once it's done the file is complete.
  const useLive = state === "running" && !!liveItems;

  const [read, setRead] = useState<{ id: string; t: Transcript | null; error: string | null } | null>(null);
  useEffect(() => {
    if (!agentId || useLive) return;
    let cancelled = false;
    getChatSubagentTranscript(t.chatId, agentId, 200).then(
      (tr) => !cancelled && setRead({ id: agentId, t: tr, error: null }),
      (e: unknown) => !cancelled && setRead({ id: agentId, t: null, error: String(e) }),
    );
    return () => {
      cancelled = true;
    };
  }, [t.chatId, agentId, useLive]);
  const loaded = read?.id === agentId ? read : null;
  const items = useLive ? liveItems : (loaded?.t?.items ?? liveItems ?? []);
  const omitted = useLive ? 0 : (loaded?.t?.omitted ?? 0);
  const a = call?.result?.agent;
  const stats = agentStats(a);
  const ms = a?.durationMs;

  return (
    <>
      <div className="tr-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="sheet tr-panel" role="dialog" aria-modal="true" aria-labelledby="subagent-title" tabIndex={-1}>
        <header className="tr-bar">
          <span>Subagent</span>
          {agentId && (
            <>
              <span className="tr-bar-sep" aria-hidden>
                ·
              </span>
              <span className="mono tr-bar-id ellipsis">{agentId}</span>
            </>
          )}
          <button type="button" className="icon-btn tr-close" aria-label="Close" title="Close (Esc)" onClick={onClose}>
            ✕
          </button>
        </header>
        <div className="tr-body">
          <div className="tr-hero">
            <h2 id="subagent-title" className="tr-title">
              {t.title}
            </h2>
            <div className="tr-chips">
              <span className={`tr-chip tr-chip-status tone-${STATE_TONE[state]}`}>
                <span className={`dot dot-sm ${state === "running" ? "pulse" : ""}`} aria-hidden />
                {STATE_LABEL[state]}
              </span>
              {(a?.agentType ?? meta?.agentType) && <span className="tr-chip mono">{a?.agentType ?? meta?.agentType}</span>}
              {(loaded?.t?.model ?? meta?.model) && <span className="tr-chip mono">{(loaded?.t?.model ?? meta?.model ?? "").replace(/^claude-/, "")}</span>}
              {ms != null && <span className="tr-chip num">{formatDuration(ms)}</span>}
              {stats && <span className="tr-chip num">{stats}</span>}
            </div>
          </div>
          <section className="tr-section tr-conv" aria-label="Transcript" aria-live={useLive ? "polite" : undefined}>
            {omitted > 0 && (
              <div className="tr-omitted">
                {omitted} earlier item{omitted === 1 ? "" : "s"} not shown.
              </div>
            )}
            {items.map((it, i) => (
              <Item key={omitted + i} item={it} />
            ))}
            {items.length === 0 &&
              (!agentId && !liveItems ? (
                <p className="tr-note">No transcript available for this subagent.</p>
              ) : loaded?.error ? (
                <p className="tr-note tr-error" role="alert">
                  Couldn't load the transcript: {loaded.error}
                </p>
              ) : !loaded && !useLive ? (
                <p className="tr-note">Loading transcript…</p>
              ) : (
                <p className="tr-note">No messages yet.</p>
              ))}
            {useLive && (
              <div className="tr-next">
                <span className="dot dot-sm pulse tone-accent" aria-hidden />
                Working…
              </div>
            )}
          </section>
        </div>
      </div>
    </>
  );
}
