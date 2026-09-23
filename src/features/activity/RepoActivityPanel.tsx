import { useEffect, useState } from "react";
import { repoActivity } from "./api";
import type { RepoActivity, SessionActivity, SubagentActivity } from "./types";
import "./activity.css";

const POLL_MS = 3000;

export interface RepoActivityPanelProps {
  /** Ruta absoluta del repo (se incluyen sus worktrees). */
  repoPath: string;
  /** Abrir el detalle de una sesión en background (p. ej. RunDetailView). */
  onOpenSession?: (session: SessionActivity) => void;
}

const basename = (p: string | null) => (p ? p.replace(/\/+$/, "").split("/").pop() || p : "");

function ago(ms: number | null, now: number): string {
  if (ms == null) return "—";
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  return `${h}h ${m % 60}m ago`;
}

const KIND_LABEL: Record<string, string> = { interactive: "Interactive", background: "Background", unlisted: "Headless" };

type Tone = "accent" | "ok" | "warn" | "danger" | "muted";

function sessionState(s: SessionActivity): { label: string; tone: Tone; live: boolean } {
  if (s.state === "blocked" || s.status === "waiting") {
    return { label: s.waitingFor ? `Waiting · ${s.waitingFor}` : "Waiting", tone: "warn", live: true };
  }
  if (s.state === "working" || s.status === "busy") return { label: "Working", tone: "accent", live: true };
  if (s.alive) return { label: "Idle", tone: "muted", live: false };
  if (s.state === "stopped") return { label: "Stopped", tone: "danger", live: false };
  return { label: "Finished", tone: "ok", live: false };
}

function ToolLine({ name, summary }: { name: string | null; summary: string | null }) {
  if (!name) return null;
  return (
    <span className="act-tool ellipsis" title={summary ? `${name} · ${summary}` : name}>
      <span className="act-tool-name">{name}</span>
      {summary && <span className="act-tool-sum"> {summary}</span>}
    </span>
  );
}

function SessionRow({ s, now, onOpen }: { s: SessionActivity; now: number; onOpen?: (s: SessionActivity) => void }) {
  const st = sessionState(s);
  const label = s.name ?? `${KIND_LABEL[s.kind] ?? s.kind} session`;
  const openable = onOpen && s.kind === "background";
  const body = (
    <>
      <span className={`dot tone-${st.tone} ${st.live ? "pulse" : ""}`} aria-hidden />
      <span className="act-main">
        <span className="act-line">
          <span className="act-name ellipsis">{label}</span>
          <span className="act-chip">{KIND_LABEL[s.kind] ?? s.kind}</span>
          {s.isAppRun && <span className="act-chip act-chip-app">App run</span>}
        </span>
        <span className="act-line act-sub">
          <span className={`act-state tone-${st.tone}`}>{st.label}</span>
          {s.cwd && <span className="act-path mono ellipsis" title={s.cwd}>{basename(s.cwd)}</span>}
          <ToolLine name={s.lastTool} summary={s.lastToolSummary} />
        </span>
      </span>
      <span className="act-time num">{ago(s.lastActivityAt ?? s.startedAt, now)}</span>
    </>
  );
  return openable ? (
    <button type="button" className="act-row" onClick={() => onOpen(s)} title={`Open ${label}`}>
      {body}
    </button>
  ) : (
    <div className="act-row" title={s.sessionId}>
      {body}
    </div>
  );
}

function SubagentRow({ a, now }: { a: SubagentActivity; now: number }) {
  const tone: Tone = a.active ? "accent" : a.finished ? "ok" : "muted";
  const state = a.active ? "Working" : a.finished ? "Finished" : "No recent activity";
  const where = a.worktree ? basename(a.worktree) : basename(a.cwd);
  const owner = a.sessionName ?? `${KIND_LABEL[a.sessionKind] ?? a.sessionKind} session`;
  return (
    <div className={`act-row ${a.active ? "" : "act-row-dim"}`} title={`${a.agentId} · session ${a.sessionId}`}>
      <span className={`dot tone-${tone} ${a.active ? "pulse" : ""}`} aria-hidden />
      <span className="act-main">
        <span className="act-line">
          <span className="act-name ellipsis">{a.description ?? a.agentId}</span>
          {a.agentType && <span className="act-chip">{a.agentType}</span>}
          {a.workflowPhase && <span className="act-chip">{a.workflowPhase}</span>}
          {a.sessionIsAppRun && <span className="act-chip act-chip-app">App run</span>}
        </span>
        <span className="act-line act-sub">
          <span className={`act-state tone-${tone}`}>{state}</span>
          {where && (
            <span className="act-path mono ellipsis" title={a.worktree ?? a.cwd ?? ""}>
              {a.worktree ? "⎇ " : ""}
              {where}
            </span>
          )}
          <span className="act-owner ellipsis">from {owner}</span>
          <ToolLine name={a.lastTool} summary={a.lastToolSummary} />
        </span>
      </span>
      <span className="act-time num">{ago(a.lastActivityAt, now)}</span>
    </div>
  );
}

/**
 * Todo lo que Claude Code está haciendo en un repo: sesiones interactivas, en
 * background y headless, y subagentes de cualquier sesión que trabajen dentro del repo
 * o sus worktrees. Hace polling cada 3 s solo mientras está montado.
 */
export function RepoActivityPanel({ repoPath, onOpenSession }: RepoActivityPanelProps) {
  const [data, setData] = useState<RepoActivity | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    setData(null);
    setError(null);
    const loop = async () => {
      try {
        const d = await repoActivity(repoPath);
        if (cancelled) return;
        setData(d);
        setError(null);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
      if (!cancelled) timer = setTimeout(loop, POLL_MS);
    };
    void loop();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [repoPath]);

  const now = data?.generatedAt ?? Date.now();
  const liveSessions = data?.sessions.filter((s) => s.alive) ?? [];
  const pastSessions = data?.sessions.filter((s) => !s.alive) ?? [];
  const activeAgents = data?.subagents.filter((a) => a.active) ?? [];
  const pastAgents = data?.subagents.filter((a) => !a.active) ?? [];
  const working = liveSessions.filter((s) => sessionState(s).live).length + activeAgents.length;

  return (
    <section className="act-panel" aria-label={`Claude activity in ${basename(repoPath)}`}>
      <header className="act-head">
        <h3 className="act-title">Activity</h3>
        <span className={`act-live ${working ? "on" : ""}`}>
          <span className={`dot dot-sm ${working ? "tone-accent pulse" : "tone-muted"}`} aria-hidden />
          {data ? (working ? `${working} working` : "Quiet") : "Loading…"}
        </span>
      </header>
      {error && (
        <div className="banner banner-error" role="alert">
          {error}
        </div>
      )}
      {data && data.sessions.length === 0 && data.subagents.length === 0 && (
        <p className="act-empty">No Claude sessions or agents in this repository right now.</p>
      )}
      {liveSessions.length > 0 && (
        <div className="act-group">
          <h4 className="section-label">Sessions · {liveSessions.length}</h4>
          {liveSessions.map((s) => (
            <SessionRow key={s.sessionId} s={s} now={now} onOpen={onOpenSession} />
          ))}
        </div>
      )}
      {activeAgents.length > 0 && (
        <div className="act-group">
          <h4 className="section-label">Agents · {activeAgents.length}</h4>
          {activeAgents.map((a) => (
            <SubagentRow key={`${a.sessionId}:${a.agentId}`} a={a} now={now} />
          ))}
        </div>
      )}
      {(pastSessions.length > 0 || pastAgents.length > 0) && (
        <details className="act-group act-past">
          <summary className="section-label">Inactive · {pastSessions.length + pastAgents.length}</summary>
          {pastSessions.map((s) => (
            <SessionRow key={s.sessionId} s={s} now={now} onOpen={onOpenSession} />
          ))}
          {pastAgents.map((a) => (
            <SubagentRow key={`${a.sessionId}:${a.agentId}`} a={a} now={now} />
          ))}
        </details>
      )}
    </section>
  );
}
