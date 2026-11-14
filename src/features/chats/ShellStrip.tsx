import { useEffect, useState } from "react";
import { stopChatShell } from "../../domain/api";
import { formatDuration } from "../../lib/format";
import { useToast } from "../../ui/Toasts";
import type { Shell } from "./stream";

/**
 * The chat's running shells, above the composer: each with its command, how long it has run and
 * a Stop. A background shell stops alone (`stop_task`); a foreground one stops with the turn.
 */
export function ShellStrip({ chatId, shells, onStopTurn }: { chatId: string; shells: Record<string, Shell>; onStopTurn: () => void }) {
  const list = Object.values(shells);
  const [now, setNow] = useState(Date.now);
  const [stopping, setStopping] = useState<ReadonlySet<string>>(new Set());
  const toast = useToast();
  useEffect(() => {
    if (!list.length) return;
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [list.length]);
  if (!list.length) return null;

  const stop = (sh: Shell) => {
    if (!sh.taskId) return onStopTurn();
    setStopping((s) => new Set(s).add(sh.toolUseId));
    // The row leaves once Claude Code reports the task ended; until then it stays "stopping".
    stopChatShell(chatId, sh.taskId).catch((e: unknown) => {
      setStopping((s) => new Set([...s].filter((id) => id !== sh.toolUseId)));
      toast("Couldn't stop the command", String(e), "danger");
    });
  };
  const reveal = (id: string) =>
    document.querySelector(`[data-tool-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });

  return (
    <ul className="chat-shells" aria-label="Running commands">
      {list.map((sh) => {
        const busy = stopping.has(sh.toolUseId);
        return (
          <li key={sh.toolUseId} className="chat-shell">
            <button type="button" className="chat-shell-main" title={sh.command} onClick={() => reveal(sh.toolUseId)}>
              <span className="chat-shell-dot" aria-hidden />
              <code className="chat-shell-cmd ellipsis">{sh.command}</code>
              {sh.description && <span className="chat-shell-desc ellipsis">{sh.description}</span>}
              <span className="chat-shell-time num">{busy ? "Stopping…" : formatDuration(now - sh.startedAt)}</span>
            </button>
            <button type="button" className="btn btn-xs btn-ghost" aria-label={`Stop ${sh.command}`} disabled={busy} onClick={() => stop(sh)}>
              Stop
            </button>
          </li>
        );
      })}
    </ul>
  );
}
