import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import type { Executor, Finish, Isolation, Repo } from "../../domain/types";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { executorLabel } from "../executors";
import { FINISH_HINT, FINISH_LABEL, ISOLATION_HINT, ISOLATION_LABEL } from "./status";
import "./tasks.css";

/** How one run delivers its work. `isolation` is absent for workflows (they manage their own worktree). */
export interface RunConfig {
  isolation?: Isolation;
  finish: Finish;
  review: boolean;
}

export interface AskLaunchOptions {
  /** Task key (or title) shown in the header. */
  name: string;
  repo: Repo | null | undefined;
  /** Effective executor of the run: a workflow hides Isolation. */
  executor: Executor;
  /** Element to anchor to; centered when absent. */
  anchor?: HTMLElement | null;
}

type AskLaunch = (opts: AskLaunchOptions) => Promise<RunConfig | null>;

const FINISHES: Finish[] = ["changes", "commit", "pr"];
const ISOLATIONS: Isolation[] = ["worktree", "in_place"];

/** Preselection: the repo defaults only; per-task values are deliberately ignored. */
export function launchPreset(repo: Repo | null | undefined, executor: Executor): RunConfig {
  const finish = repo?.defaultFinish ?? "pr";
  const review = repo?.defaultReview ?? true;
  return executor.kind === "workflow" ? { finish, review } : { isolation: repo?.defaultIsolation ?? "worktree", finish, review };
}

const LaunchContext = createContext<AskLaunch>(() => {
  console.error("useAskLaunch() outside <LaunchConfigProvider>: the launch was cancelled.");
  return Promise.resolve(null);
});

/** `const config = await askLaunch({ name, repo, executor })` → `null` if the user cancelled. */
export function useAskLaunch(): AskLaunch {
  return useContext(LaunchContext);
}

interface Pending extends AskLaunchOptions {
  id: number;
  resolve: (c: RunConfig | null) => void;
}

export function LaunchConfigProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const current = useRef<Pending | null>(null);
  const seq = useRef(0);

  const ask = useCallback<AskLaunch>(
    (opts) =>
      new Promise<RunConfig | null>((resolve) => {
        current.current?.resolve(null);
        const p = { ...opts, id: ++seq.current, resolve };
        current.current = p;
        setPending(p);
      }),
    [],
  );

  const settle = useCallback((c: RunConfig | null) => {
    const p = current.current;
    current.current = null;
    setPending(null);
    p?.resolve(c);
  }, []);

  useEffect(() => () => current.current?.resolve(null), []);

  return (
    <LaunchContext.Provider value={ask}>
      {children}
      {pending && <LaunchPopover key={pending.id} {...pending} onSettle={settle} />}
    </LaunchContext.Provider>
  );
}

const GAP = 6;
const MARGIN = 12;

function LaunchPopover({ name, repo, executor, anchor, onSettle }: AskLaunchOptions & { onSettle: (c: RunConfig | null) => void }) {
  const ref = useFocusTrap<HTMLDivElement>(() => onSettle(null));
  const titleId = useId();
  const [config, setConfig] = useState<RunConfig>(() => launchPreset(repo, executor));
  const [pos, setPos] = useState<CSSProperties | null>(null);
  const isWorkflow = executor.kind === "workflow";
  const anchorRect = useRef(anchor?.isConnected ? anchor.getBoundingClientRect() : null);

  useLayoutEffect(() => {
    const a = anchorRect.current;
    const el = ref.current;
    if (!a || !el) return;
    const { width, height } = el.getBoundingClientRect();
    const below = a.bottom + GAP + height <= window.innerHeight - MARGIN;
    const top = below ? a.bottom + GAP : Math.max(MARGIN, a.top - GAP - height);
    const left = Math.min(Math.max(MARGIN, a.right - width), window.innerWidth - width - MARGIN);
    setPos({ top, left });
  }, [ref]);

  // The shell's global shortcuts ignore defaultPrevented events: nothing opens underneath.
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !/^[acvxz]$/i.test(e.key)) e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Enter" || e.nativeEvent.isComposing || (e.target as HTMLElement).dataset.cancel !== undefined) return;
    e.preventDefault();
    e.stopPropagation();
    onSettle(config);
  };

  const anchored = anchorRect.current !== null;

  return (
    <>
      <div className={anchored ? "lp-scrim" : "confirm-scrim"} onClick={() => onSettle(null)} aria-hidden />
      <div
        ref={ref}
        className={`lp ${anchored ? "lp-anchored" : "lp-centered"}`}
        style={anchored ? (pos ?? { visibility: "hidden" }) : undefined}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <div className="lp-head">
          <span id={titleId} className="lp-title">
            Run <span className="mono">{name}</span>
          </span>
          <span className="tk-muted ellipsis">
            {executorLabel(executor)}
            {repo ? ` in ${repo.name}` : ""}
          </span>
        </div>

        <div className="lp-body">
          {!isWorkflow && (
            <>
              <span className="lp-label">Isolation</span>
              <div className="lp-opt">
                <Seg
                  label="Isolation"
                  options={ISOLATIONS.map((i) => [i, ISOLATION_LABEL[i]])}
                  value={config.isolation ?? "worktree"}
                  onPick={(isolation) => setConfig((c) => ({ ...c, isolation }))}
                />
                <span className="tk-hint">{ISOLATION_HINT[config.isolation ?? "worktree"]}</span>
              </div>
            </>
          )}

          <span className="lp-label">Finish</span>
          <div className="lp-opt">
            <Seg
              label="Finish"
              options={FINISHES.map((f) => [f, FINISH_LABEL[f]])}
              value={config.finish}
              onPick={(finish) => setConfig((c) => ({ ...c, finish }))}
            />
            <span className="tk-hint">{FINISH_HINT[config.finish]}</span>
          </div>

          <span className="lp-label">Review</span>
          <div className="lp-opt">
            <button
              type="button"
              role="switch"
              aria-checked={config.review}
              className="nt-toggle"
              onClick={() => setConfig((c) => ({ ...c, review: !c.review }))}
            >
              <span className={`nt-track ${config.review ? "on" : ""}`} aria-hidden>
                <span className="nt-knob" />
              </span>
              Review before In Review
            </button>
            <span className="tk-hint">
              {config.review
                ? `${repo?.reviewer ?? "The reviewer"} checks the criteria first, read-only`
                : "Goes straight to In Review"}
            </span>
          </div>
        </div>

        <div className="lp-foot">
          <button type="button" className="btn btn-ghost btn-sm" data-cancel onClick={() => onSettle(null)}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary btn-sm lp-run" data-autofocus onClick={() => onSettle(config)}>
            <span aria-hidden>▶</span>
            Run
            <span className="nt-kbd" aria-hidden>
              ↵
            </span>
          </button>
        </div>
      </div>
    </>
  );
}

function Seg<T extends string>({
  label,
  options,
  value,
  onPick,
}: {
  label: string;
  options: [T, string][];
  value: T;
  onPick: (v: T) => void;
}) {
  return (
    <div className="segmented tk-seg" role="radiogroup" aria-label={label}>
      {options.map(([v, l]) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={value === v}
          className={`tk-seg-opt ${value === v ? "on" : ""}`}
          onClick={() => onPick(v)}
        >
          {l}
        </button>
      ))}
    </div>
  );
}
