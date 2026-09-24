import { useEffect, useMemo, useState } from "react";
import { useRuns } from "../../domain/hooks/runs";
import type { Repo } from "../../domain/types";
import { repoActivity } from "./api";
import type { RepoActivity, SessionActivity, SubagentActivity } from "./types";
import "./activity.css";

const POLL_MS = 3000;

type Tone = "accent" | "ok" | "warn" | "danger" | "muted";

const KIND: Record<string, { label: string; letter: string; cls: string }> = {
  interactive: { label: "Interactive", letter: "I", cls: "k-interactive" },
  background: { label: "Background", letter: "B", cls: "k-background" },
  unlisted: { label: "Headless", letter: "U", cls: "k-unlisted" },
};
const kindOf = (k: string) => KIND[k] ?? { label: k, letter: "?", cls: "k-unlisted" };

const basename = (p: string | null) => (p ? p.replace(/\/+$/, "").split("/").pop() || p : "");

function ago(ms: number | null, now: number): string {
  if (ms == null) return "—";
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ago`;
  const h = Math.floor(m / 60);
  if (h < 48) return `${h}h ${m % 60}m ago`;
  return `${Math.floor(h / 24)}d ago`;
}

function sessionState(s: SessionActivity): { label: string; tone: Tone; live: boolean } {
  if (s.state === "blocked" || s.status === "waiting") {
    return { label: s.waitingFor ? `Waiting · ${s.waitingFor}` : "Waiting", tone: "warn", live: true };
  }
  if (s.state === "working" || s.status === "busy") return { label: "Working", tone: "accent", live: true };
  if (s.alive) return { label: "Idle", tone: "muted", live: false };
  if (s.state === "stopped") return { label: "Stopped", tone: "danger", live: false };
  return { label: "Ended", tone: "muted", live: false };
}

function useRepoActivity(repoPath: string | null): { data: RepoActivity | null; error: string | null } {
  const [state, setState] = useState<{ path: string; data: RepoActivity | null; error: string | null } | null>(null);
  useEffect(() => {
    if (!repoPath) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;
    const loop = async () => {
      try {
        const d = await repoActivity(repoPath);
        if (!cancelled) setState({ path: repoPath, data: d, error: null });
      } catch (e) {
        if (!cancelled) setState((prev) => ({ path: repoPath, data: prev?.path === repoPath ? prev.data : null, error: String(e) }));
      }
      if (!cancelled) timer = setTimeout(loop, POLL_MS);
    };
    void loop();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [repoPath]);
  if (!repoPath || state?.path !== repoPath) return { data: null, error: null };
  return { data: state.data, error: state.error };
}

interface SessionGroup {
  session: SessionActivity;
  subs: SubagentActivity[];
}

/** Subagentes debajo de su sesión; los de sesiones que no figuran arman una propia. */
function groupSessions(data: RepoActivity): SessionGroup[] {
  const groups = new Map<string, SessionGroup>(data.sessions.map((s) => [s.sessionId, { session: s, subs: [] }]));
  for (const a of data.subagents) {
    let g = groups.get(a.sessionId);
    if (!g) {
      g = {
        session: {
          sessionId: a.sessionId,
          id: null,
          kind: a.sessionKind,
          name: a.sessionName,
          cwd: a.sessionCwd,
          status: null,
          state: null,
          waitingFor: null,
          startedAt: null,
          pid: null,
          alive: false,
          isAppRun: a.sessionIsAppRun,
          lastActivityAt: a.lastActivityAt,
          lastTool: null,
          lastToolSummary: null,
          entrypoint: null,
        },
        subs: [],
      };
      groups.set(a.sessionId, g);
    }
    g.subs.push(a);
  }
  return [...groups.values()];
}

const isActiveGroup = (g: SessionGroup) => g.session.alive || g.subs.some((a) => a.active);

export interface ActivityViewProps {
  /** `null`: repos de todos los proyectos. */
  projectId: string | null;
  /** Abrir el run de Nodal de una sesión lanzada por la app. */
  onOpenRun?: (runId: string) => void;
}

/**
 * Pantalla "Activity": todo lo que Claude Code está haciendo en cada repo del proyecto
 * (sesiones interactivas, en background y headless, y sus subagentes), lo haya lanzado
 * Nodal o no.
 */
export function ActivityView({ projectId, onOpenRun }: ActivityViewProps) {
  const runs = useRuns({ projectId });
  const repos = useMemo(
    () =>
      [...runs.repos.values()]
        .filter((r) => !projectId || r.projectId === projectId)
        .sort((a, b) => a.position - b.position || a.name.localeCompare(b.name)),
    [runs.repos, projectId],
  );
  const [picked, setPicked] = useState<string | null>(null);
  const repo: Repo | undefined = repos.find((r) => r.id === picked) ?? repos[0];
  const { data, error } = useRepoActivity(repo?.path ?? null);

  const runBySession = useMemo(() => {
    const m = new Map<string, string>();
    for (const v of runs.byId.values()) if (v.run.sessionId) m.set(v.run.sessionId, v.run.id);
    return m;
  }, [runs.byId]);

  if (runs.loaded && repos.length === 0) {
    return (
      <div className="activity-view">
        <div className="center-state">
          <div className="center-state-body">
            <div className="center-state-title">No repos yet</div>
            <div className="center-state-text">Add a repo to this project to see what Claude is doing in it.</div>
          </div>
        </div>
      </div>
    );
  }

  const now = data?.generatedAt ?? Date.now();
  const groups = data ? groupSessions(data) : [];
  const active = groups.filter(isActiveGroup);
  const inactive = groups.filter((g) => !isActiveGroup(g));
  const liveSessions = data?.sessions.filter((s) => s.alive).length ?? 0;
  const liveAgents = data?.subagents.filter((a) => a.active).length ?? 0;

  return (
    <div className="activity-view">
      <div className="act-bar">
        <div className="segmented act-repos" role="tablist" aria-label="Repos">
          {repos.map((r) => {
            const on = r.id === repo?.id;
            return (
              <button
                key={r.id}
                type="button"
                role="tab"
                aria-selected={on}
                className={`act-repo ${on ? "on" : ""}`}
                title={r.path}
                onClick={() => setPicked(r.id)}
              >
                {r.name}
                {on && liveSessions > 0 && <span className="act-repo-cnt">{liveSessions}</span>}
              </button>
            );
          })}
        </div>
        <span className="act-summary">
          {data
            ? `${liveSessions} live session${liveSessions === 1 ? "" : "s"} · ${liveAgents} agent${liveAgents === 1 ? "" : "s"} active · includes sessions Nodal didn't start`
            : repo
              ? "Loading…"
              : ""}
        </span>
      </div>
      <div className="act-list" role="tabpanel">
        {error && (
          <div className="banner banner-error" role="alert">
            {error}
          </div>
        )}
        {data && groups.length === 0 && (
          <div className="center-state">
            <div className="center-state-body">
              <div className="center-state-title">No Claude activity here yet</div>
              <div className="center-state-text">
                Sessions started with claude in this repo show up here, even if Nodal didn't launch them.
              </div>
            </div>
          </div>
        )}
        {[
          { name: "Active", list: active, dim: false },
          { name: "Inactive", list: inactive, dim: true },
        ]
          .filter((g) => g.list.length)
          .map((g) => (
            <section key={g.name} className="act-group" aria-label={g.name}>
              <h3 className="act-group-head">
                <span className={`dot dot-sm ${g.dim ? "tone-muted" : "tone-accent pulse"}`} aria-hidden />
                {g.name}
                <span className="act-group-cnt">{g.list.length}</span>
              </h3>
              {g.list.map((sg) => {
                const runId = runBySession.get(sg.session.sessionId);
                return (
                  <SessionCard
                    key={sg.session.sessionId}
                    group={sg}
                    now={now}
                    dim={g.dim}
                    onOpen={runId && onOpenRun ? () => onOpenRun(runId) : undefined}
                  />
                );
              })}
            </section>
          ))}
      </div>
    </div>
  );
}

function SessionCard({ group, now, dim, onOpen }: { group: SessionGroup; now: number; dim: boolean; onOpen?: () => void }) {
  const s = group.session;
  const st = sessionState(s);
  const k = kindOf(s.kind);
  const name = s.name ?? `${k.label} session`;
  const head = (
    <>
      <span className={`act-kind ${k.cls}`} aria-hidden>
        {k.letter}
      </span>
      <span className="act-main">
        <span className="act-line">
          <span className="act-name ellipsis">{name}</span>
          {s.isAppRun && <span className="act-chip act-chip-app">Nodal</span>}
          <span className="act-kind-label">{k.label}</span>
        </span>
        <span className="act-line act-toolline">
          {s.lastTool && <span className="act-tool-name">{s.lastTool}</span>}
          <span className="act-tool-sum ellipsis">{s.lastToolSummary ?? (s.cwd ? basename(s.cwd) : "")}</span>
        </span>
      </span>
      <span className={`act-state tone-${st.tone}`}>
        <span className={`dot dot-sm ${st.live && st.tone === "accent" ? "pulse" : ""}`} aria-hidden />
        {st.label}
      </span>
      <span className="act-time num">{ago(s.lastActivityAt ?? s.startedAt, now)}</span>
      <span className="act-chev" aria-hidden>
        {onOpen ? "›" : ""}
      </span>
    </>
  );
  return (
    <div className={`act-card ${dim ? "act-card-dim" : ""}`}>
      {onOpen ? (
        <button type="button" className="act-head-row" onClick={onOpen} title={`Open run for ${name}`}>
          {head}
        </button>
      ) : (
        <div className="act-head-row" title={s.sessionId}>
          {head}
        </div>
      )}
      {group.subs.length > 0 && (
        <div className="act-subs">
          {group.subs.map((a) => (
            <SubagentRow key={a.agentId} a={a} now={now} />
          ))}
        </div>
      )}
    </div>
  );
}

function SubagentRow({ a, now }: { a: SubagentActivity; now: number }) {
  const tone: Tone = a.active ? "accent" : a.finished ? "ok" : "muted";
  const wt = a.worktree ? basename(a.worktree) : null;
  return (
    <div className="act-sub" title={`${a.agentId} · ${a.active ? "Working" : a.finished ? "Finished" : "No recent activity"}`}>
      <span className={`dot tone-${tone} ${a.active ? "pulse" : ""}`} aria-hidden />
      <span className="act-sub-type ellipsis">{a.agentType ?? "agent"}</span>
      <span className="act-sub-desc">
        <span className="ellipsis">{a.description ?? a.agentId}</span>
        {wt && <span className="act-sub-wt ellipsis">⎇ {wt}</span>}
        {a.workflowPhase && <span className="act-chip">{a.workflowPhase}</span>}
      </span>
      <span className="act-sub-tool">
        {a.lastTool && <span className={a.active ? "act-tool-name" : "act-tool-idle"}>{a.lastTool}</span>}
        <span className="act-tool-sum ellipsis">{a.lastToolSummary ?? ""}</span>
      </span>
      <span className="act-time num">{ago(a.lastActivityAt, now)}</span>
    </div>
  );
}
