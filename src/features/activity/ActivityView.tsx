import { useMemo, useState } from "react";
import { projectActivity, type ProjectActivity } from "../../domain/api";
import { useAllRuns } from "../../domain/hooks/runs";
import { POLL, usePolled, useRepos } from "../../domain/hooks/store";
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
  // Same engine as the rest of the data layer: one timer per repo, paused while the window is hidden.
  const { data, error } = usePolled<RepoActivity>(
    repoPath ? `activity:${repoPath}` : null,
    () => repoActivity(repoPath as string),
    [],
    POLL_MS,
  );
  return { data: data ?? null, error };
}

/** Per-repo counts for the project (a single `claude agents`), for the tabs. */
function useProjectActivity(projectId: string | null) {
  return usePolled<ProjectActivity>(
    projectId ? `project-activity:${projectId}` : null,
    () => projectActivity(projectId as string),
    [],
    POLL.live * 2,
  ).data;
}

interface SessionGroup {
  session: SessionActivity;
  subs: SubagentActivity[];
}

/** Subagents under their session; those from unlisted sessions get one of their own. */
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
  /** `null`: repos from every project. */
  projectId: string | null;
  /** Open the Nodal run of a session launched by the app. */
  onOpenRun?: (runId: string) => void;
}

/**
 * "Activity" screen: everything Claude Code is doing in each of the project's repos
 * (interactive, background and headless sessions, and their subagents), whether Nodal
 * launched it or not.
 */
export function ActivityView({ projectId, onOpenRun }: ActivityViewProps) {
  // Already sorted by position.
  const reposQ = useRepos(projectId);
  const repos = reposQ.data ?? [];
  const runs = useAllRuns();
  const counts = useProjectActivity(projectId);
  const countByRepo = useMemo(() => new Map((counts?.repos ?? []).map((c) => [c.repoId, c])), [counts]);
  const [picked, setPicked] = useState<string | null>(null);
  const repo: Repo | undefined = repos.find((r) => r.id === picked) ?? repos[0];
  const { data, error } = useRepoActivity(repo?.path ?? null);

  const runBySession = useMemo(() => {
    const m = new Map<string, string>();
    for (const r of runs.data ?? []) if (r.sessionId && !m.has(r.sessionId)) m.set(r.sessionId, r.id);
    return m;
  }, [runs.data]);

  if (reposQ.data && repos.length === 0) {
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
            // Working/waiting sessions and active subagents (same criteria as `project_activity`).
            const c = countByRepo.get(r.id);
            const n = c?.sessions ?? 0;
            const agents = c?.agents ?? 0;
            return (
              <button
                key={r.id}
                type="button"
                role="tab"
                aria-selected={on}
                className={`act-repo ${on ? "on" : ""}`}
                title={`${r.path}${n || agents ? ` · ${n} session${n === 1 ? "" : "s"}, ${agents} agent${agents === 1 ? "" : "s"}` : ""}`}
                onClick={() => setPicked(r.id)}
              >
                {r.name}
                {n > 0 && (
                  <>
                    <span className="act-repo-cnt" aria-hidden>
                      {n}
                    </span>
                    <span className="sr-only">, {n} active session{n === 1 ? "" : "s"}</span>
                  </>
                )}
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
