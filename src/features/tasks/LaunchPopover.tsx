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
import { Kbd } from "../../ui/Kbd";
import { ExecutorName } from "../executors";
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
  /** Task title; when set, `name` is shown as the key above it. */
  title?: string;
  repo: Repo | null | undefined;
  /** Effective executor of the run: a workflow hides Isolation. */
  executor: Executor;
  /** Element to anchor to; centered when absent. */
  anchor?: HTMLElement | null;
}

type AskLaunch = (opts: AskLaunchOptions) => Promise<RunConfig | null>;

const FINISHES: Finish[] = ["changes", "commit", "pr"];
const ISOLATIONS: Isolation[] = ["worktree", "in_place"];

/**
 * Preselection: the repo defaults only; per-task values are deliberately ignored. Review always
 * starts off; turn it on per run.
 */
export function launchPreset(repo: Repo | null | undefined, executor: Executor): RunConfig {
  const finish = repo?.defaultFinish ?? "pr";
  const review = false;
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

function LaunchPopover({ name, title, repo, executor, anchor, onSettle }: AskLaunchOptions & { onSettle: (c: RunConfig | null) => void }) {
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
  // Not visibility:hidden: that makes the popover unfocusable and useFocusTrap's autofocus is lost.

  return (
    <>
      <div className={anchored ? "lp-scrim" : "confirm-scrim"} onClick={() => onSettle(null)} aria-hidden />
      <div
        ref={ref}
        className={`lp ${anchored ? "lp-anchored" : "lp-centered"}`}
        style={anchored ? (pos ?? { opacity: 0 }) : undefined}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        tabIndex={-1}
        onKeyDown={onKeyDown}
      >
        <div className="lp-head">
          <div className="lp-crumbs">
            {title && <span className="mono">{name}</span>}
            {title && repo && (
              <span className="lp-sep" aria-hidden>
                ·
              </span>
            )}
            {repo && <span className="mono ellipsis">{repo.name}</span>}
            <button type="button" className="icon-btn lp-close" data-cancel aria-label="Cancel" onClick={() => onSettle(null)}>
              ✕
            </button>
          </div>
          <span id={titleId} className="lp-title">
            <span className="sr-only">Run </span>
            {title ?? name}
          </span>
        </div>

        <div className="lp-body">
          <span className="lp-label">Executor</span>
          <div className="lp-opt lp-exec">
            <ExecutorName executor={executor} />
          </div>

          {!isWorkflow && (
            <>
              <span className="lp-label">Where</span>
              <div className="lp-opt">
                <Seg
                  label="Where it works"
                  options={ISOLATIONS.map((i) => [i, ISOLATION_LABEL[i]])}
                  value={config.isolation ?? "worktree"}
                  onPick={(isolation) => setConfig((c) => ({ ...c, isolation }))}
                />
                <span className="lp-hint">{ISOLATION_HINT[config.isolation ?? "worktree"]}</span>
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
            <span className="lp-hint">{FINISH_HINT[config.finish]}</span>
          </div>

          <span className="lp-label">Review</span>
          <div className="lp-opt">
            <button
              type="button"
              role="switch"
              aria-checked={config.review}
              className="lp-toggle"
              onClick={() => setConfig((c) => ({ ...c, review: !c.review }))}
            >
              <span className="switch" aria-checked={config.review} aria-hidden />
              Review before In Review
            </button>
            <span className="lp-hint">
              {config.review
                ? `${repo?.reviewer ?? "The reviewer"} checks the criteria first, read-only`
                : "Goes straight to In Review"}
            </span>
          </div>
        </div>

        <div className="lp-foot">
          <button type="button" className="btn btn-ghost" data-cancel onClick={() => onSettle(null)}>
            Cancel
          </button>
          <button type="button" className="btn btn-primary lp-run" data-autofocus onClick={() => onSettle(config)}>
            Run
            <Kbd aria-hidden>↵</Kbd>
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
    <div className="seg" role="radiogroup" aria-label={label}>
      {options.map(([v, l]) => (
        <button
          key={v}
          type="button"
          role="radio"
          aria-checked={value === v}
          className="seg-opt"
          onClick={() => onPick(v)}
        >
          {l}
        </button>
      ))}
    </div>
  );
}
