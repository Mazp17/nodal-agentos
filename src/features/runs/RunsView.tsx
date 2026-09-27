import { useEffect, useMemo, useRef, useState, type KeyboardEvent } from "react";
import { useQueueSummary, useRuns } from "../../domain/hooks/runs";
import { formatDateTime } from "../../lib/format";
import { readPref, writePref } from "../../shell/storage";
import { EmptyState } from "../../ui/EmptyState";
import { ExecutorAvatar } from "../executors";
import { useRunActions } from "./actions";
import { useExternalSessions } from "./external";
import { buildRows, formatLaunch, GROUPS, sortGroup, type RunGroup, type RunRow, type RunSource } from "./rows";
import "./runs.css";

type Tab = "all" | RunGroup;

const TABS: { id: Tab; label: string }[] = [
  { id: "all", label: "All" },
  ...GROUPS.map((g) => ({ id: g.id, label: g.label })),
];

const TAB_PREF = "runsTab";

const SOURCES: { value: RunSource; label: string; hint: string }[] = [
  { value: "all", label: "All", hint: "Nodal runs and external sessions" },
  { value: "nodal", label: "Nodal", hint: "Only runs Nodal started" },
  { value: "external", label: "External", hint: "Claude sessions started outside Nodal" },
];

const EMPTY: Record<Tab, { title: string; description: string }> = {
  all: { title: "No runs yet", description: "Run a task from the board, or start claude in one of these repos." },
  needs: { title: "Nothing needs you", description: "Runs waiting for input and failed runs show up here." },
  running: { title: "Nothing running", description: "Run a task from the board and it shows up here." },
  queued: { title: "Nothing queued", description: "Runs wait here while every slot is busy." },
  done: { title: "No finished runs yet", description: "Runs land here once they finish." },
};

/** Finished rows shown on "All" before "Show all". */
const FINISHED_PREVIEW = 5;

export interface RunsViewProps {
  /** `null`: all projects. The queue is always global. */
  projectId: string | null;
  onOpenRun: (runId: string) => void;
  onGoToBoard: () => void;
}

/** "Runs" screen: Nodal's runs and the sessions started outside Nodal, grouped by what they need. */
export function RunsView({ projectId, onOpenRun, onGoToBoard }: RunsViewProps) {
  const state = useRuns({ projectId });
  const summary = useQueueSummary();
  const ext = useExternalSessions(projectId);
  const actions = useRunActions();

  const [tab, setTabState] = useState<Tab>(() => {
    const saved = readPref(TAB_PREF);
    return TABS.some((t) => t.id === saved) ? (saved as Tab) : "all";
  });
  const setTab = (next: Tab) => {
    setTabState(next);
    writePref(TAB_PREF, next);
  };
  const [source, setSource] = useState<RunSource>("all");
  const [projectFilter, setProjectFilter] = useState<string | null>(null);
  const [repoFilter, setRepoFilter] = useState<string | null>(null);
  const [collapsed, setCollapsed] = useState<ReadonlySet<RunGroup>>(new Set());
  const [finishedAll, setFinishedAll] = useState(false);

  const fProject = projectId ? null : projectFilter;
  const repoOptions = [...state.repos.values()].filter((r) => (projectId ? r.projectId === projectId : !fProject || r.projectId === fProject));
  const fRepo = repoFilter && repoOptions.some((r) => r.id === repoFilter) ? repoFilter : null;

  const { views, projects, repos, tasks, latestByTask, now } = state;
  const rows = useMemo(
    () =>
      buildRows(views, ext.data, source, { projects, repos, tasks, latestByTask, now }).filter(
        (r) => (!fProject || r.projectId === fProject) && (!fRepo || r.repoId === fRepo),
      ),
    [views, projects, repos, tasks, latestByTask, now, ext.data, source, fProject, fRepo],
  );
  const byGroup = new Map(GROUPS.map((g) => [g.id, sortGroup(rows.filter((r) => r.group === g.id), g.id)]));
  const count = (t: Tab) => (t === "all" ? rows.length : (byGroup.get(t)?.length ?? 0));
  const groups = GROUPS.filter((g) => (tab === "all" || tab === g.id) && (byGroup.get(g.id)?.length ?? 0) > 0);

  // ▲ swaps with the previous visible queued run; the rest of the global queue keeps its order.
  const queueIds = state.queue.map((v) => v.run.id);
  const visibleQueue = (byGroup.get("queued") ?? []).filter((r) => r.view).map((r) => r.id);
  const moveUp = (id: string) => {
    const k = visibleQueue.indexOf(id);
    if (k <= 0) return;
    const a = queueIds.indexOf(visibleQueue[k - 1]);
    const b = queueIds.indexOf(id);
    if (a < 0 || b < 0) return;
    const next = [...queueIds];
    [next[a], next[b]] = [next[b], next[a]];
    void actions.reorder(next);
  };

  const toggle = (g: RunGroup) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(g)) next.delete(g);
      else next.add(g);
      return next;
    });

  const showProject = projectId === null;
  const hasFilters = fProject !== null || fRepo !== null;
  const slots = Math.min(Math.max(summary.capacity, summary.running), 16);

  return (
    <div className="runs">
      <div className="runs-tabs">
        <div className="runs-tablist" role="tablist" aria-label="Runs">
          {TABS.map((t) => (
            <button
              key={t.id}
              id={`runs-tab-${t.id}`}
              type="button"
              role="tab"
              aria-selected={tab === t.id}
              className={`runs-tab ${tab === t.id ? "on" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
              <span className="runs-tab-count num">{count(t.id)}</span>
            </button>
          ))}
        </div>
        <div className="runs-filters">
          {hasFilters && (
            <button
              type="button"
              className="btn btn-sm btn-ghost runs-clear"
              onClick={() => {
                setProjectFilter(null);
                setRepoFilter(null);
              }}
            >
              Clear
            </button>
          )}
          {showProject && (
            <Picker
              label="Project"
              value={fProject}
              options={[
                { value: null, label: "All projects" },
                ...[...state.projects.values()].map((p) => ({ value: p.id, label: p.name, dot: p.color })),
              ]}
              onChange={(v) => {
                setProjectFilter(v);
                setRepoFilter(null);
              }}
            />
          )}
          <Picker
            label="Repo"
            value={fRepo}
            options={[{ value: null, label: "All repos" }, ...repoOptions.map((r) => ({ value: r.id, label: r.name, mono: true }))]}
            onChange={setRepoFilter}
          />
          <Picker
            label="Source"
            value={source}
            wide
            options={SOURCES}
            onChange={(v) => setSource(v ?? "all")}
          />
        </div>
      </div>
      {state.error && <div className="banner banner-error runs-error">{state.error}</div>}
      {state.liveError && <div className="banner banner-warn runs-error">{state.liveError}</div>}
      {ext.error && source !== "nodal" && (
        <div className="banner banner-warn runs-error">Couldn't read sessions started outside Nodal: {ext.error}</div>
      )}
      <div className="table-head runs-cols">
        <span>Task</span>
        <span>Run by</span>
        <span>Progress</span>
        <span>Launched</span>
        <span>Result</span>
      </div>
      <div className="runs-rows" role="tabpanel" aria-labelledby={`runs-tab-${tab}`}>
        {groups.map((g) => {
          const list = byGroup.get(g.id) ?? [];
          const open = !collapsed.has(g.id);
          const cut = tab === "all" && g.id === "done" && list.length > FINISHED_PREVIEW;
          const shown = cut && !finishedAll ? list.slice(0, FINISHED_PREVIEW) : list;
          return (
            <section key={g.id} className={`runs-group tone-${g.tone}`} aria-label={g.label}>
              <div className="runs-group-head">
                <button type="button" className="runs-group-toggle" aria-expanded={open} onClick={() => toggle(g.id)}>
                  <span className="runs-group-chev" aria-hidden>
                    {open ? "▾" : "▸"}
                  </span>
                  <span className="dot" aria-hidden />
                  {g.label}
                  <span className="runs-group-count num">{list.length}</span>
                </button>
                {g.id === "running" && slots > 0 && (
                  <span className="runs-slots" title={`${summary.running} of ${summary.capacity} slots in use`}>
                    <span className="runs-slots-bar" aria-hidden>
                      {Array.from({ length: slots }, (_, k) => (
                        <span key={k} className={k < summary.running ? "on" : ""} />
                      ))}
                    </span>
                    {summary.running}/{summary.capacity} slots
                  </span>
                )}
                {g.hint && <span className="runs-group-hint ellipsis">{g.hint}</span>}
              </div>
              {open &&
                shown.map((r) => (
                  <Row
                    key={r.id}
                    row={r}
                    now={state.now}
                    showProject={showProject}
                    queueIndex={r.group === "queued" ? visibleQueue.indexOf(r.id) : -1}
                    busy={actions.busy}
                    onOpen={() => onOpenRun(r.id)}
                    onMoveUp={() => moveUp(r.id)}
                    onRemove={() => r.view && void actions.remove(r.view, r.taskKey ?? r.title)}
                    onConfirm={() => r.view && void actions.confirm(r.view, r.taskKey ?? r.title)}
                  />
                ))}
              {open && cut && (
                <button type="button" className="btn btn-xs btn-ghost runs-more" onClick={() => setFinishedAll(!finishedAll)}>
                  {finishedAll ? "Show fewer" : `Show all ${list.length}`}
                </button>
              )}
            </section>
          );
        })}
        {groups.length === 0 &&
          (state.loaded ? (
            <EmptyState className="runs-empty-state" title={EMPTY[tab].title} description={EMPTY[tab].description}>
              <button type="button" className="btn" onClick={onGoToBoard}>
                Go to board
              </button>
            </EmptyState>
          ) : (
            <div className="runs-empty">Loading runs…</div>
          ))}
      </div>
    </div>
  );
}

interface RowProps {
  row: RunRow;
  now: number;
  showProject: boolean;
  /** Position among the visible queued runs; `-1` when not queued. */
  queueIndex: number;
  busy: boolean;
  onOpen: () => void;
  onMoveUp: () => void;
  onRemove: () => void;
  onConfirm: () => void;
}

function Row({ row: r, now, showProject, queueIndex, busy, onOpen, onMoveUp, onRemove, onConfirm }: RowProps) {
  const external = r.view === null;
  const queued = r.group === "queued" && !external;
  const awaiting = r.view?.phase === "awaiting";
  return (
    <div className={`runs-cols runs-row ${external ? "external" : ""}`}>
      <span className="runs-task">
        <span className="runs-task-line">
          {r.taskKey && <span className="runs-key">{r.taskKey}</span>}
          {external ? (
            <span className="runs-title ellipsis">{r.title}</span>
          ) : (
            // Stretched over the row: the whole row opens the run, the queue buttons sit above it.
            <button type="button" className="runs-title runs-open ellipsis" onClick={onOpen}>
              {r.title}
            </button>
          )}
          {r.clash && (
            <span className="runs-clash" role="img" title={r.clash} aria-label={r.clash}>
              ⚠
            </span>
          )}
        </span>
        <span className="runs-where">
          {showProject && (
            <>
              <span className="proj-swatch" style={{ background: r.project?.color ?? "var(--text-disabled)" }} aria-hidden />
              <span>{r.project?.name ?? "—"}</span>
              <span aria-hidden>·</span>
            </>
          )}
          <span className="runs-repo ellipsis">{r.repoName}</span>
        </span>
      </span>
      <span className="runs-by" title={r.byTitle}>
        {r.executor ? <ExecutorAvatar executor={r.executor} /> : <span className="runs-ext-avatar" aria-hidden />}
        <span className="runs-by-name ellipsis">{r.by}</span>
      </span>
      <span className={`runs-progress tone-${r.tone}`}>
        {r.pct !== null ? (
          <span className="runs-progress-line">
            <span className="runs-bar">
              <span style={{ width: `${r.pct}%` }} />
            </span>
            <span className="runs-phase-text">{r.progress}</span>
          </span>
        ) : (
          <span className="runs-doing ellipsis">{r.progress}</span>
        )}
        {r.meta && <span className="runs-meta num">{r.meta}</span>}
      </span>
      <span className="runs-launched num" title={r.launchedAt != null ? formatDateTime(r.launchedAt) : "Waiting for a free slot"}>
        {r.launchedAt != null ? formatLaunch(r.launchedAt, now) : "Not started"}
      </span>
      <span className="runs-result-cell">
        <span className={`runs-result tone-${r.tone}`}>
          <span className={`dot dot-sm ${r.pulse ? "pulse" : ""}`} aria-hidden />
          <span className="ellipsis">{r.label}</span>
        </span>
        {queued && (
          <span className="runs-queue-actions">
            {awaiting && (
              <button type="button" className="btn btn-xs btn-amber" disabled={busy} onClick={onConfirm}>
                Confirm
              </button>
            )}
            <button
              type="button"
              className="icon-btn runs-queue-btn"
              aria-label={`Move ${r.taskKey ?? r.title} up`}
              title="Move up"
              disabled={busy || queueIndex <= 0}
              onClick={onMoveUp}
            >
              ▲
            </button>
            <button
              type="button"
              className="icon-btn runs-queue-btn"
              aria-label={`Remove ${r.taskKey ?? r.title} from queue`}
              title="Remove from queue"
              disabled={busy}
              onClick={onRemove}
            >
              ✕
            </button>
          </span>
        )}
      </span>
    </div>
  );
}

interface PickerOption<T extends string> {
  value: T | null;
  label: string;
  hint?: string;
  dot?: string;
  mono?: boolean;
}

/** Toolbar filter: "Label  Value ▾" with a menu. */
function Picker<T extends string>({
  label,
  value,
  options,
  wide,
  onChange,
}: {
  label: string;
  value: T | null;
  options: PickerOption<T>[];
  /** Two-line options (label and hint). */
  wide?: boolean;
  onChange: (value: T | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const wrap = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const current = options.find((o) => o.value === value);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!wrap.current?.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    wrap.current?.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const items = [...(wrap.current?.querySelectorAll<HTMLElement>(".menu-item") ?? [])];
    const i = items.indexOf(document.activeElement as HTMLElement);
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      close();
    } else if (e.key === "ArrowDown") {
      e.preventDefault();
      items[Math.min(items.length - 1, i + 1)]?.focus();
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      items[Math.max(0, i - 1)]?.focus();
    } else if (e.key === "Tab") {
      setOpen(false);
    }
  };

  // Every picker's "all" option reads "All" on the trigger.
  const shown = value === null ? "All" : (current?.label ?? "All");

  return (
    <div className="runs-picker" ref={wrap}>
      <button
        ref={trigger}
        type="button"
        className="btn btn-sm btn-ghost runs-picker-trigger"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => setOpen(!open)}
      >
        <span className="runs-picker-label">{label}</span>
        <span className="runs-picker-value">{shown}</span>
        <span className="runs-picker-caret" aria-hidden>
          ▾
        </span>
      </button>
      {open && (
        <div className={`menu runs-picker-menu ${wide ? "wide" : ""}`} role="menu" aria-label={label} onKeyDown={onKey}>
          {options.map((o) => (
            <button
              key={o.value ?? "__all"}
              type="button"
              role="menuitemradio"
              aria-checked={o.value === value}
              className={`menu-item ${o.hint ? "runs-picker-item-2" : ""}`}
              onClick={() => {
                onChange(o.value);
                close();
              }}
            >
              <span className="menu-mark" aria-hidden>
                {o.value === value ? "✓" : ""}
              </span>
              {o.dot !== undefined && <span className="proj-swatch" style={{ background: o.dot }} aria-hidden />}
              {o.hint ? (
                <span className="runs-picker-text">
                  <span>{o.label}</span>
                  <span className="runs-picker-hint">{o.hint}</span>
                </span>
              ) : (
                <span className={`ellipsis ${o.mono && o.value !== null ? "mono" : ""}`}>{o.label}</span>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
