import { useState, type FormEvent } from "react";
import type { AppConfig, Issue } from "../linear/api";
import { formatDuration, formatTokens } from "../../lib/format";
import { useToast } from "../../ui/Toasts";
import type { RunActions } from "./actions";
import { launchRun } from "./api";
import { pickRepoFolder, tildify, useHome } from "./folders";
import type { RunKind, RunView } from "./status";
import type { RunsState } from "./useRuns";
import "./runs.css";

export type RunsTab = "active" | "queued" | "done" | "failed";

const TABS: { id: RunsTab; label: string; kinds: RunKind[] }[] = [
  { id: "active", label: "Active", kinds: ["running", "starting"] },
  { id: "queued", label: "Queued", kinds: ["queued"] },
  { id: "done", label: "Finished", kinds: ["done"] },
  { id: "failed", label: "Failed", kinds: ["failed"] },
];

const EMPTY: Record<RunsTab, string> = {
  active: "Nothing running. Launch an issue from the board.",
  queued: "Nothing queued.",
  done: "No finished runs yet.",
  failed: "No failed runs.",
};

function phaseOf(v: RunView): { text: string; pct: number } {
  const n = v.phaseTotal;
  if (v.kind === "queued") return { text: "Waiting", pct: 0 };
  if (v.kind === "starting") return { text: "Starting", pct: 0 };
  if (v.kind === "done") return { text: n ? `${n}/${n}${v.phaseName ? ` · ${v.phaseName}` : ""}` : "Done", pct: 100 };
  if (v.phaseIndex == null || !n) return { text: v.kind === "failed" ? "—" : "Running", pct: 0 };
  const pct = Math.round((v.phaseIndex / n) * 100);
  if (v.kind === "failed") return { text: `Stopped at ${v.phaseName ?? v.phaseIndex}`, pct };
  return { text: `${v.phaseIndex}/${n}${v.phaseName ? ` · ${v.phaseName}` : ""}`, pct };
}

interface Props {
  runs: RunsState;
  issues: Map<string, Issue>;
  config: AppConfig;
  actions: RunActions;
  tab: RunsTab;
  onTab: (t: RunsTab) => void;
  onOpenRun: (key: string) => void;
}

export function RunsView({ runs, issues, config, actions, tab, onTab, onOpenRun }: Props) {
  const [manual, setManual] = useState(false);
  const counts = Object.fromEntries(TABS.map((t) => [t.id, runs.views.filter((v) => t.kinds.includes(v.kind)).length]));
  const kinds = TABS.find((t) => t.id === tab)!.kinds;
  const rows = runs.views.filter((v) => kinds.includes(v.kind));
  // Los bloqueados esperando al usuario no ocupan slot (igual que en issue_runs/store.rs).
  const active = runs.views.filter((v) => (v.kind === "running" && !v.waitingFor) || v.kind === "starting");
  const queue = runs.views
    .filter((v) => v.kind === "queued")
    .sort((a, b) => (a.ir?.queuedAt ?? 0) - (b.ir?.queuedAt ?? 0));
  const slots = Math.max(config.concurrency, active.length);

  return (
    <div className="runs">
      <div className="runs-main">
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
              onClick={() => onTab(t.id)}
            >
              {t.label}
              <span className="runs-tab-count num">{counts[t.id]}</span>
            </button>
          ))}
          </div>
          <button
            type="button"
            className="btn btn-sm runs-manual-toggle"
            aria-expanded={manual}
            onClick={() => setManual(!manual)}
          >
            {manual ? "Close" : "Launch manually"}
          </button>
        </div>
        {manual && <ManualLaunch config={config} onLaunched={() => void runs.refresh()} />}
        {runs.error && <div className="banner banner-error">{runs.error}</div>}
        <div className="table-head runs-cols">
          <span>Issue</span>
          <span>Title</span>
          <span>Project</span>
          <span>Phase</span>
          <span className="text-right">Duration</span>
          <span className="text-right">Tokens</span>
          <span>Result</span>
        </div>
        <div className="runs-rows" role="tabpanel" aria-labelledby={`runs-tab-${tab}`}>
          {rows.map((v) => {
            const issue = v.issueId ? issues.get(v.issueId) : undefined;
            const ph = phaseOf(v);
            return (
              <button key={v.key} type="button" className="runs-cols runs-row" onClick={() => onOpenRun(v.key)}>
                <span className="runs-issue">{v.identifier ?? "—"}</span>
                <span className="ellipsis">{issue?.title ?? v.run?.name ?? v.workflow ?? "Manual run"}</span>
                <span className="runs-proj ellipsis">{issue?.project?.name ?? (v.issueId ? "—" : "Manual")}</span>
                <span className={`runs-phase tone-${v.tone}`}>
                  <span className="runs-phase-text ellipsis">{ph.text}</span>
                  <span className="runs-bar">
                    <span style={{ width: `${ph.pct}%` }} />
                  </span>
                </span>
                <span className="text-right num runs-num">{v.kind === "queued" ? "—" : formatDuration(v.durationMs)}</span>
                <span className="text-right num runs-num">{formatTokens(v.tokens)}</span>
                <span>
                  <span className={`badge tone-${v.tone}`}>
                    <span className="dot dot-sm" aria-hidden />
                    {v.kind === "running" && !v.waitingFor ? "Running" : v.label}
                  </span>
                </span>
              </button>
            );
          })}
          {rows.length === 0 && <div className="runs-empty">{EMPTY[tab]}</div>}
        </div>
      </div>

      <aside className="queue" aria-labelledby="queue-title">
        <div className="queue-head">
          <h2 id="queue-title" className="queue-title">
            Queue
          </h2>
          <span className="queue-cap">
            {active.length}/{config.concurrency} slots in use
          </span>
        </div>
        <div className="queue-slots" style={{ gridTemplateColumns: `repeat(${Math.min(slots, 16)}, 1fr)` }} aria-hidden>
          {Array.from({ length: Math.min(slots, 16) }, (_, k) => (
            <span key={k} className={k < active.length ? "on" : ""} />
          ))}
        </div>
        {queue.map((v, k) => {
          const issue = v.issueId ? issues.get(v.issueId) : undefined;
          return (
            <div key={v.key} className="queue-item">
              <span className="queue-pos num" title={v.ir?.status === "launching" ? "Launching" : undefined}>
                {v.ir?.status === "launching" ? "→" : v.queuePos ?? k + 1}
              </span>
              <button type="button" className="queue-open" onClick={() => onOpenRun(v.key)}>
                <span className="queue-issue">{v.identifier}</span>
                <span className="queue-t ellipsis">{issue?.title ?? v.workflow}</span>
              </button>
              {v.ir?.status === "queued" && (
                <button
                  type="button"
                  className="icon-btn queue-rm"
                  aria-label={`Remove ${v.identifier} from queue`}
                  disabled={actions.busy}
                  onClick={() => void actions.cancel(v)}
                >
                  ✕
                </button>
              )}
            </div>
          );
        })}
        {queue.length === 0 && (
          <p className="queue-empty">Queue is empty. New runs start immediately while slots are free.</p>
        )}
      </aside>
    </div>
  );
}

// Opción del select que abre el selector nativo de carpetas.
const OTHER_FOLDER = "\u0000other";

/** Lanzar `claude --bg` a mano en un repo, sin issue. */
function ManualLaunch({ config, onLaunched }: { config: AppConfig; onLaunched: () => void }) {
  const toast = useToast();
  const home = useHome();
  const [cwd, setCwd] = useState(config.repos[0]?.path ?? "");
  const [picked, setPicked] = useState<string[]>([]);
  const [prompt, setPrompt] = useState("");
  const [launching, setLaunching] = useState(false);
  const paths = [...new Set([...config.repos.map((r) => r.path), ...picked])];

  const chooseFolder = async () => {
    try {
      const res = await pickRepoFolder(cwd || null);
      if (!res) return;
      setPicked((p) => (p.includes(res.path) ? p : [...p, res.path]));
      setCwd(res.path);
    } catch (err) {
      toast("Couldn't open the folder picker", String(err), "danger");
    }
  };

  const onSubmit = async (e: FormEvent) => {
    e.preventDefault();
    setLaunching(true);
    try {
      const ref = await launchRun(cwd, prompt);
      toast("Run launched", `${ref.id} · ${tildify(ref.cwd, home)}`, "ok");
      setPrompt("");
      onLaunched();
    } catch (err) {
      toast("Couldn't launch the run", String(err), "danger");
    } finally {
      setLaunching(false);
    }
  };

  return (
    <form className="runs-manual" onSubmit={onSubmit}>
      {/* Carpeta elegida con el selector nativo (o un repo mapeado): sin texto libre. */}
      <select
        className="input input-mono runs-manual-cwd"
        aria-label="Folder to run in"
        value={cwd}
        onChange={(e) => {
          if (e.target.value === OTHER_FOLDER) void chooseFolder();
          else setCwd(e.target.value);
        }}
      >
        {!cwd && <option value="">Choose a folder…</option>}
        {paths.map((p) => (
          <option key={p} value={p}>
            {tildify(p, home)}
          </option>
        ))}
        <option value={OTHER_FOLDER}>Other folder…</option>
      </select>
      <input
        className="input runs-manual-prompt"
        aria-label="Prompt"
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
        placeholder="/skill args"
        spellCheck={false}
      />
      <button type="submit" className="btn btn-primary btn-lg" disabled={launching || !cwd || !prompt.trim()}>
        {launching ? "Launching…" : "Launch"}
      </button>
    </form>
  );
}
