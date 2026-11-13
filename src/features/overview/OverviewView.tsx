import { useEffect, useState } from "react";
import type { Chat, Project } from "../../domain/types";
import { chatTitle, shortAgo } from "../chats/model";
import { useFinishedAt, useRunStates, useUnreadChats } from "../chats/stream";
import "./overview.css";

const LIST_MAX = 5;
/** "Just finished" looks back this far; older endings are just history. */
const FINISHED_WINDOW = 24 * 60 * 60 * 1000;

interface Props {
  chats: Chat[];
  projects: Project[];
  onOpen: (chat: Chat) => void;
}

/** What the agents are doing: working now, just finished, and the latest chats. */
export function OverviewView({ chats, projects, onOpen }: Props) {
  const states = useRunStates();
  const finished = useFinishedAt();
  const unread = useUnreadChats();
  const now = useNow();
  const projectById = new Map(projects.map((p) => [p.id, p]));
  const byId = new Map(chats.map((c) => [c.id, c]));

  const working = chats.filter((c) => states.get(c.id) === "busy");
  const done = [...finished]
    .filter(([id, at]) => states.get(id) !== "busy" && now - at < FINISHED_WINDOW && byId.has(id))
    .sort((a, b) => b[1] - a[1])
    .slice(0, LIST_MAX)
    .map(([id, at]) => ({ chat: byId.get(id)!, at }));
  const recent = [...chats].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, LIST_MAX);

  const row = (c: Chat, meta: React.ReactNode, tone: "live" | "done" | "idle") => {
    const p = projectById.get(c.projectId);
    return (
      <li key={c.id}>
        <button type="button" className="ov-row" onClick={() => onOpen(c)}>
          <span className={`ov-state ov-${tone}`} aria-hidden />
          <span className="ov-main">
            <span className="ov-title ellipsis">
              {chatTitle(c)}
              {unread.has(c.id) && <span className="ov-new">New</span>}
            </span>
            <span className="ov-sub ellipsis">
              {p && <span className="project-dot" style={{ ["--project-color" as string]: p.color }} aria-hidden />}
              {p?.name ?? "Unknown project"}
              <span aria-hidden>·</span>
              <span className="ov-agent">{c.agent ? `@${c.agent}` : "Claude"}</span>
            </span>
          </span>
          <span className="ov-meta num">{meta}</span>
        </button>
      </li>
    );
  };

  return (
    <div className="ov">
      <section className="ov-section" aria-labelledby="ov-working">
        <h2 id="ov-working" className="ov-h">
          Working now <span className="ov-count num">{working.length || ""}</span>
        </h2>
        {working.length === 0 ? (
          <p className="ov-empty">No agent is working right now.</p>
        ) : (
          <ul className="ov-list">{working.map((c) => row(c, <span className="ov-live-label">Working</span>, "live"))}</ul>
        )}
      </section>

      <section className="ov-section" aria-labelledby="ov-done">
        <h2 id="ov-done" className="ov-h">Just finished</h2>
        {done.length === 0 ? (
          <p className="ov-empty">Nothing finished in the last 24 hours.</p>
        ) : (
          <ul className="ov-list">{done.map(({ chat, at }) => row(chat, `${shortAgo(at, now)} ago`, "done"))}</ul>
        )}
      </section>

      <section className="ov-section" aria-labelledby="ov-recent">
        <h2 id="ov-recent" className="ov-h">Recent chats</h2>
        {recent.length === 0 ? (
          <p className="ov-empty">No chats yet. Start one with the + next to a project.</p>
        ) : (
          <ul className="ov-list">
            {recent.map((c) => row(c, shortAgo(c.updatedAt, now), states.get(c.id) === "busy" ? "live" : "idle"))}
          </ul>
        )}
      </section>
    </div>
  );
}

/** Re-render every 30s so "3m ago" stays true. */
function useNow(): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(t);
  }, []);
  return now;
}
