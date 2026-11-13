import { useEffect, useState, type ReactNode } from "react";
import { DropdownMenu } from "radix-ui";
import { claudeUsage, respondChatPermission, type UsageSummary, type UsageTotals } from "../../domain/api";
import { POLL, usePolled } from "../../domain/hooks/store";
import type { Chat, Project } from "../../domain/types";
import { formatTokens } from "../../lib/format";
import { readJsonPref, writePref } from "../../shell/storage";
import { useToast } from "../../ui/Toasts";
import { chatTitle, shortAgo } from "../chats/model";
import { lastAnswer, useChatNeeds, useFinishedAt, useRunStates, useUnreadChats, type ChatNeed } from "../chats/stream";
import "./overview.css";

const DAY = 24 * 60 * 60 * 1000;
const FINISHED_MAX = 8;

type Period = "24h" | "7d" | "30d" | "all";
const PERIODS: { id: Period; label: string; long: string }[] = [
  { id: "24h", label: "24h", long: "Last 24 hours" },
  { id: "7d", label: "7d", long: "Last 7 days" },
  { id: "30d", label: "30d", long: "Last 30 days" },
  { id: "all", label: "All", long: "All time" },
];
const PERIOD_PREF = "overviewPeriod";
const USAGE_OPEN_PREF = "overviewUsageOpen";
const isPeriod = (v: unknown): v is Period => PERIODS.some((p) => p.id === v);
const isBool = (v: unknown): v is boolean => typeof v === "boolean";

/** Glyphs on the app's 14px grid and stroke. */
const GLYPH = {
  need: <path d="M7 2.2 12.4 11.6H1.6zM7 6v2.4M7 10v.1" />,
  done: <path d="M2.6 7.4 5.6 10.2 11.4 3.8" />,
  tokens: <path d="M2.5 11.5V8.5M5.5 11.5V6.5M8.5 11.5V4.5M11.5 11.5V2.5" />,
  plus: <path d="M7 3v8M3 7h8" />,
  chev: <path d="M5 3.5 8.5 7 5 10.5" />,
  board: (
    <>
      <rect x="1.8" y="2.2" width="3" height="9.6" rx="1" />
      <rect x="5.5" y="2.2" width="3" height="6.6" rx="1" />
      <rect x="9.2" y="2.2" width="3" height="4.4" rx="1" />
    </>
  ),
} satisfies Record<string, ReactNode>;
type GlyphName = keyof typeof GLYPH;

const Glyph = ({ name, className }: { name: GlyphName; className?: string }) => (
  <svg className={className} width="14" height="14" viewBox="0 0 14 14" fill="none" stroke="currentColor" strokeWidth="1.3" strokeLinecap="round" strokeLinejoin="round" aria-hidden>
    {GLYPH[name]}
  </svg>
);

interface Props {
  chats: Chat[];
  projects: Project[];
  onOpen: (chat: Chat) => void;
  /** The project's chats: its latest one, or a new one when it has none. */
  onOpenProject: (projectId: string) => void;
  onOpenBoard: (projectId: string) => void;
  onNewChat: (projectId: string) => void;
  onNewProject: () => void;
}

/** Where work starts: what waits for you and what just came back first; projects and usage after. */
export function OverviewView({ chats, projects, onOpen, onOpenProject, onOpenBoard, onNewChat, onNewProject }: Props) {
  const states = useRunStates();
  const finished = useFinishedAt();
  const unread = useUnreadChats();
  const needs = useChatNeeds();
  const now = useNow();
  const live = projects.filter((p) => !p.archivedAt);
  const projectById = new Map(projects.map((p) => [p.id, p]));
  const byId = new Map(chats.map((c) => [c.id, c]));

  // Approvals first: they block a running agent.
  const needing = chats
    .filter((c) => needs.has(c.id))
    .sort((a, b) => Number(needs.get(b.id)!.kind === "approval") - Number(needs.get(a.id)!.kind === "approval"));
  const working = chats.filter((c) => states.get(c.id) === "busy" && needs.get(c.id)?.kind !== "approval");
  const done = [...finished]
    .filter(([id, at]) => states.get(id) !== "busy" && now - at < DAY && byId.has(id) && !needs.has(id))
    .sort((a, b) => b[1] - a[1])
    .map(([id, at]) => ({ chat: byId.get(id)!, at }));

  const where = (c: Chat) => {
    const p = projectById.get(c.projectId);
    return (
      <span className="ov-sub ellipsis">
        {p && <span className="project-dot" style={{ ["--project-color" as string]: p.color }} aria-hidden />}
        {p?.name ?? "Unknown project"}
        <span aria-hidden>·</span>
        <span className="ov-agent">{c.agent ? `@${c.agent}` : "Claude"}</span>
      </span>
    );
  };

  const summary = [
    needing.length ? `${needing.length} need${needing.length === 1 ? "s" : ""} you` : "nothing waits for you",
    `${done.length} finished today`,
    working.length ? `${working.length} working` : null,
  ].filter(Boolean);

  return (
    <div className="ov">
      <header className="ov-head">
        <div className="ov-head-text">
          <h1 className="ov-h1">{greeting(now)}</h1>
          <p className="ov-lede">{summary.join(" · ")}</p>
        </div>
        <NewChatMenu projects={live} onNewChat={onNewChat} onNewProject={onNewProject} />
      </header>

      <div className="ov-inbox">
        <Panel id="ov-needs" glyph="need" title="Needs you" count={needing.length} tone="need">
          {needing.length === 0 ? (
            <p className="ov-empty">Nothing is waiting on you. Approvals and failed answers land here.</p>
          ) : (
            <ul className="ov-list">
              {needing.map((c) => (
                <NeedRow key={c.id} chat={c} need={needs.get(c.id)!} where={where(c)} onOpen={() => onOpen(c)} />
              ))}
            </ul>
          )}
        </Panel>

        <Panel id="ov-done" glyph="done" title="Just finished" count={done.length}>
          {done.length === 0 ? (
            <p className="ov-empty">Nothing finished in the last 24 hours.</p>
          ) : (
            <ul className="ov-list">
              {done.slice(0, FINISHED_MAX).map(({ chat, at }) => {
                const answer = lastAnswer(chat.id);
                return (
                  <li key={chat.id}>
                    <button type="button" className="ov-row" onClick={() => onOpen(chat)}>
                      <span className={`ov-state ${unread.has(chat.id) ? "ov-done" : "ov-idle"}`} aria-hidden />
                      <span className="ov-main">
                        <span className="ov-title ellipsis">
                          {chatTitle(chat)}
                          {unread.has(chat.id) && <span className="ov-new">New</span>}
                        </span>
                        {answer && <span className="ov-answer">{answer}</span>}
                        {where(chat)}
                      </span>
                      <span className="ov-meta num">{shortAgo(at, now)} ago</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </Panel>
      </div>

      {working.length > 0 && (
        <section className="ov-strip" aria-labelledby="ov-working">
          <h2 id="ov-working" className="ov-strip-h">
            Working
          </h2>
          <ul className="ov-chips">
            {working.map((c) => {
              const p = projectById.get(c.projectId);
              return (
                <li key={c.id}>
                  <button type="button" className="ov-chip" onClick={() => onOpen(c)} title={p ? `${p.name} · ${chatTitle(c)}` : chatTitle(c)}>
                    <span className="ov-state ov-live" aria-hidden />
                    <span className="ellipsis">{chatTitle(c)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      <section className="ov-strip" aria-labelledby="ov-projects">
        <h2 id="ov-projects" className="ov-strip-h">
          Projects
        </h2>
        <ul className="ov-chips">
          {live.map((p) => {
            const own = chats.filter((c) => c.projectId === p.id);
            const need = own.filter((c) => needs.has(c.id)).length;
            const busy = own.some((c) => states.get(c.id) === "busy");
            return (
              <li key={p.id} className="ov-project" style={{ ["--project-color" as string]: p.color }}>
                <button type="button" className="ov-chip ov-project-open" onClick={() => onOpenProject(p.id)} title={`Open ${p.name}'s chats`}>
                  <span className={`ov-project-dot ${busy ? "busy" : ""}`} aria-hidden />
                  <span className="ellipsis">{p.name}</span>
                  {need > 0 && (
                    <span className="ov-chip-badge num" aria-label={`${need} need you`}>
                      {need}
                    </span>
                  )}
                </button>
                <button type="button" className="ov-chip ov-project-board" onClick={() => onOpenBoard(p.id)} title={`${p.name}'s board`} aria-label={`Open ${p.name}'s board`}>
                  <Glyph name="board" />
                </button>
              </li>
            );
          })}
          <li>
            <button type="button" className="ov-chip ov-chip-ghost" onClick={onNewProject}>
              <Glyph name="plus" />
              New project
            </button>
          </li>
        </ul>
      </section>

      <UsageStrip now={now} />
    </div>
  );
}

function greeting(now: number): string {
  const h = new Date(now).getHours();
  return h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 19 ? "Good afternoon" : "Good evening";
}

function NewChatMenu({ projects, onNewChat, onNewProject }: { projects: Project[]; onNewChat: (id: string) => void; onNewProject: () => void }) {
  if (projects.length === 0) {
    return (
      <button type="button" className="btn btn-primary" onClick={onNewProject}>
        <Glyph name="plus" />
        New project
      </button>
    );
  }
  return (
    <DropdownMenu.Root>
      <DropdownMenu.Trigger className="btn btn-primary ov-new-chat">
        <Glyph name="plus" />
        New chat
      </DropdownMenu.Trigger>
      <DropdownMenu.Portal>
        <DropdownMenu.Content className="menu ov-menu" align="end" sideOffset={6} collisionPadding={8}>
          <DropdownMenu.Label className="menu-label">Start a chat in</DropdownMenu.Label>
          {projects.map((p) => (
            <DropdownMenu.Item key={p.id} className="menu-item" onSelect={() => onNewChat(p.id)}>
              <span className="project-dot" style={{ ["--project-color" as string]: p.color }} aria-hidden />
              {p.name}
            </DropdownMenu.Item>
          ))}
        </DropdownMenu.Content>
      </DropdownMenu.Portal>
    </DropdownMenu.Root>
  );
}

/** A chat waiting on you: approvals are answered right here; failures open the chat. */
function NeedRow({ chat, need, where, onOpen }: { chat: Chat; need: ChatNeed; where: ReactNode; onOpen: () => void }) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  // A new request in the same chat gets its buttons back.
  const reqId = need.kind === "approval" ? need.requestId : null;
  useEffect(() => setBusy(false), [reqId]);

  const answer = (allow: boolean) => {
    if (!reqId) return;
    setBusy(true);
    respondChatPermission(chat.id, reqId, allow).then(
      // The row goes away (or moves to the next request) with `permissionResolved`.
      () => {},
      (e: unknown) => {
        setBusy(false);
        toast("Couldn't answer the prompt", String(e), "danger");
      },
    );
  };

  return (
    <li className="ov-need-row">
      <button type="button" className="ov-row" onClick={onOpen}>
        <span className={`ov-state ${need.kind === "approval" ? "ov-need" : "ov-failed"}`} aria-hidden />
        <span className="ov-main">
          <span className="ov-title ellipsis">{chatTitle(chat)}</span>
          {where}
        </span>
      </button>
      <div className="ov-need-body">
        {need.kind === "approval" ? (
          <>
            <code className="ov-ask ellipsis" title={need.summary ?? undefined}>
              <span className="ov-ask-tool">{need.tool}</span>
              {need.summary && <span className="ov-ask-arg">{need.summary}</span>}
            </code>
            <div className="ov-actions">
              <button type="button" className="btn btn-sm btn-amber" disabled={busy} onClick={() => answer(true)}>
                Approve
              </button>
              <button type="button" className="btn btn-sm" disabled={busy} onClick={() => answer(false)}>
                Deny
              </button>
              <button type="button" className="btn btn-sm btn-ghost" onClick={onOpen}>
                Open
              </button>
              {need.more > 0 && <span className="ov-more num">+{need.more} more</span>}
            </div>
          </>
        ) : (
          <>
            <p className="ov-fail">{need.message}</p>
            <div className="ov-actions">
              <button type="button" className="btn btn-sm" onClick={onOpen}>
                Open chat
              </button>
            </div>
          </>
        )}
      </div>
    </li>
  );
}

function Panel(p: { id: string; glyph: GlyphName; title: string; count?: number; tone?: string; children: ReactNode }) {
  return (
    <section className={`ov-panel ${p.tone && p.count ? `ov-panel-${p.tone}` : ""}`} aria-labelledby={p.id}>
      <h2 id={p.id} className="ov-panel-head">
        <span className="ov-panel-glyph">
          <Glyph name={p.glyph} />
        </span>
        <span className="ov-panel-title">{p.title}</span>
        {!!p.count && <span className="ov-panel-count num">{p.count}</span>}
      </h2>
      <div className="ov-panel-body">{p.children}</div>
    </section>
  );
}

/** Claude Code's tokens on this Mac: one line folded, the detail when opened. */
function UsageStrip({ now }: { now: number }) {
  const usage = usePolled<UsageSummary>("claudeUsage", claudeUsage, [], POLL.slow);
  const [period, setPeriodState] = useState<Period>(() => readJsonPref(PERIOD_PREF, "7d", isPeriod));
  const [open, setOpenState] = useState(() => readJsonPref(USAGE_OPEN_PREF, false, isBool));
  const setPeriod = (p: Period) => {
    setPeriodState(p);
    writePref(PERIOD_PREF, JSON.stringify(p));
  };
  const toggle = () => {
    setOpenState(!open);
    writePref(USAGE_OPEN_PREF, JSON.stringify(!open));
  };
  const u = usage.data;
  const totals = u ? totalsOf(u, period) : null;
  const long = PERIODS.find((p) => p.id === period)!.long;
  const total30 = u ? sum(u.last30d) : 0;
  const top = u?.models30d[0];

  return (
    <section className={`ov-usage-strip ${open ? "open" : ""}`} aria-labelledby="ov-usage">
      <div className="ov-usage-bar">
        <button type="button" className="ov-usage-toggle" aria-expanded={open} aria-controls="ov-usage-detail" onClick={toggle}>
          <Glyph name="chev" className="ov-usage-chev" />
          <span id="ov-usage" className="ov-strip-h">
            Usage
          </span>
          {usage.error && !u ? (
            <span className="ov-usage-note">Couldn't read Claude Code's sessions</span>
          ) : !u || !totals ? (
            <span className="ov-usage-note">Reading Claude Code's sessions…</span>
          ) : (
            <>
              <span className="ov-usage-n num">{formatTokens(sum(totals))}</span>
              <span className="ov-usage-note">tokens · {long.toLowerCase()}</span>
              <Spark days={u.daily30d} />
              {top && total30 > 0 && (
                <span className="ov-usage-note ellipsis">
                  {top.model} {Math.round((sum(top.totals) / total30) * 100)}%
                </span>
              )}
            </>
          )}
        </button>
        <div className="ov-periods" role="group" aria-label="Usage period">
          {PERIODS.map((p) => (
            <button key={p.id} type="button" className={`ov-period ${period === p.id ? "on" : ""}`} aria-pressed={period === p.id} title={p.long} onClick={() => setPeriod(p.id)}>
              {p.label}
            </button>
          ))}
        </div>
      </div>
      {open && u && totals && (
        <div id="ov-usage-detail">
          <UsageDetail u={u} totals={totals} long={long} now={now} />
        </div>
      )}
    </section>
  );
}

function Spark({ days }: { days: number[] }) {
  const peak = Math.max(...days, 1);
  return (
    <span className="ov-spark" aria-hidden>
      {days.map((n, i) => (
        <span key={i} className={i === days.length - 1 ? "today" : ""} style={{ height: `${n ? Math.max((n / peak) * 100, 10) : 6}%` }} />
      ))}
    </span>
  );
}

/** The period's split, the last 30 days day by day, and the models behind them. */
function UsageDetail({ u, totals, long, now }: { u: UsageSummary; totals: UsageTotals; long: string; now: number }) {
  const total = sum(totals);
  const parts = [
    { key: "out", label: "Output", n: totals.output },
    { key: "in", label: "Input", n: totals.input },
    { key: "cache", label: "Cache", n: totals.cacheRead + totals.cacheWrite },
  ];
  const peak = Math.max(...u.daily30d, 1);
  const total30 = sum(u.last30d);
  const day = (i: number) =>
    new Date(now - (u.daily30d.length - 1 - i) * DAY).toLocaleDateString(undefined, { month: "short", day: "numeric" });

  return (
    <div className="ov-usage">
      <div className="ov-usage-col">
        <div className="ov-sub-h">
          {long} · {totals.messages.toLocaleString()} responses · {u.sessions} sessions on this Mac
        </div>
        <div className="ov-split" role="img" aria-label={parts.map((p) => `${p.label} ${formatTokens(p.n)}`).join(", ")}>
          {parts.map((p) => (
            <span key={p.key} className={`ov-split-${p.key}`} style={{ flexGrow: total ? p.n / total : 0 }} />
          ))}
        </div>
        <ul className="ov-legend">
          {parts.map((p) => (
            <li key={p.key}>
              <span className={`ov-swatch ov-split-${p.key}`} aria-hidden />
              {p.label}
              <span className="ov-legend-n num">{formatTokens(p.n)}</span>
            </li>
          ))}
        </ul>
        {u.models30d.length > 0 && (
          <ul className="ov-models" aria-label="Models, last 30 days">
            {u.models30d.map((m) => {
              const share = total30 ? sum(m.totals) / total30 : 0;
              return (
                <li key={m.model} className="ov-model">
                  <span className="ov-model-name ellipsis">{m.model}</span>
                  <span className="ov-model-bar" aria-hidden>
                    <span style={{ width: `${Math.max(share * 100, 1)}%` }} />
                  </span>
                  <span className="ov-model-n num">{Math.round(share * 100)}%</span>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <div className="ov-usage-col">
        <div className="ov-sub-h">Tokens per day · last 30 days</div>
        <div className="ov-days" role="img" aria-label={`Tokens per day, last 30 days. Peak ${formatTokens(peak)}.`}>
          {u.daily30d.map((n, i) => (
            <span
              key={i}
              className={`ov-day ${i === u.daily30d.length - 1 ? "today" : ""}`}
              style={{ height: `${n ? Math.max((n / peak) * 100, 4) : 0}%` }}
              title={`${day(i)} · ${formatTokens(n)} tokens`}
            />
          ))}
        </div>
        <div className="ov-days-axis">
          <span>{day(0)}</span>
          <span>Today</span>
        </div>
      </div>
    </div>
  );
}

const sum = (t: UsageTotals) => t.input + t.output + t.cacheRead + t.cacheWrite;

function totalsOf(u: UsageSummary, p: Period): UsageTotals {
  return p === "24h" ? u.last24h : p === "7d" ? u.last7d : p === "30d" ? u.last30d : u.allTime;
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
