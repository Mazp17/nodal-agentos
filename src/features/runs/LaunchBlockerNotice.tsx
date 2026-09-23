import { useEffect, useState } from "react";
import { useToast } from "../../ui/Toasts";
import { getLaunchBlocker, openTerminalAt } from "./api";
import type { RunView } from "./status";
import { isInProgress } from "./types";
import "./launch-blocker.css";

/**
 * Motivos conocidos por los que Claude Code no deja arrancar un run, con una acción clara.
 *
 * - `trust`: `claude --bg` se niega en un repo que nunca se abrió con `claude`
 *   ("Workspace not trusted. Run `claude` in <dir> once and accept the trust prompt…").
 *   Sale sincrónico por stderr, así que llega en el error del lanzamiento (`launchError`).
 * - `workflowReview`: el workflow es nuevo o cambió y nadie lo aprobó en `/workflows`. La
 *   sesión `--bg` sí arranca: el rechazo queda como `tool_result` en su transcript y la
 *   sesión termina sin workflow ("Interrupted"). Solo se ve leyendo el transcript
 *   (`get_launch_blocker`); `claude agents` no lo reporta.
 */
export type Blocker =
  | { kind: "trust"; dir: string | null; home: boolean }
  | { kind: "workflowReview"; workflow: string | null };

const TRUST_RE = /Workspace not trusted/i;
const REVIEW_TEXT = "Review dynamic workflow before running";

/** Detecta un motivo conocido en el texto de error de un lanzamiento. */
export function classifyLaunchError(text: string | null | undefined, fallbackDir: string | null): Blocker | null {
  if (!text) return null;
  if (TRUST_RE.test(text)) {
    // "Run `claude` in <dir> once …" / "Please run `claude` in <dir> first …"
    const m = /run `claude` in (.+?) (?:once|first)\b/i.exec(text);
    return { kind: "trust", dir: m?.[1]?.trim() || fallbackDir, home: /home directory/i.test(text) };
  }
  if (text.includes(REVIEW_TEXT)) return { kind: "workflowReview", workflow: null };
  return null;
}

/** Texto para un toast de lanzamiento fallido: el motivo accionable si se reconoce. */
export function launchErrorHint(text: string | null | undefined, dir: string | null): string | undefined {
  const b = classifyLaunchError(text, dir);
  if (b?.kind === "trust" && !b.home) {
    return `Workspace not trusted. Open Claude once in ${b.dir ?? "the repository"} and accept the trust dialog (the run has a button for it).`;
  }
  return text ?? undefined;
}

// Una sesión terminada no cambia: el resultado de `get_launch_blocker` se cachea.
const blockerCache = new Map<string, Blocker | null>();

/** Terminó sin resultado del workflow: candidato a haber sido frenado por la aprobación. */
function endedWithoutResult(v: RunView): boolean {
  if (!v.run || isInProgress(v.run) || v.run.state === "stopped") return false;
  if (v.kind === "failed") return true;
  return v.kind === "done" && !v.detail?.resultStatus;
}

export function useLaunchBlocker(view: RunView | undefined): Blocker | null {
  const fromError = view ? classifyLaunchError(view.launchError, view.cwd) : null;
  const sid = view && !fromError && endedWithoutResult(view) ? view.run!.sessionId : null;
  const cwd = view?.run?.cwd ?? view?.cwd ?? "";
  const [fetched, setFetched] = useState<{ sid: string; blocker: Blocker | null } | null>(null);

  useEffect(() => {
    if (!sid) return;
    if (blockerCache.has(sid)) {
      setFetched({ sid, blocker: blockerCache.get(sid) ?? null });
      return;
    }
    let alive = true;
    getLaunchBlocker(sid, cwd)
      .then((b) => {
        const blocker: Blocker | null = b ? { kind: "workflowReview", workflow: b.workflow ?? view?.workflow ?? null } : null;
        blockerCache.set(sid, blocker);
        if (alive) setFetched({ sid, blocker });
      })
      .catch((e) => console.error("get_launch_blocker", e));
    return () => {
      alive = false;
    };
    // `view.workflow` solo completa el nombre; no hace falta repetir la consulta.
  }, [sid, cwd]);

  if (fromError) return fromError.kind === "workflowReview" ? { ...fromError, workflow: view?.workflow ?? null } : fromError;
  return sid && fetched?.sid === sid ? fetched.blocker : null;
}

const basename = (p: string) => p.replace(/\/+$/, "").split("/").pop() || p;

/** Aviso accionable para un run frenado por confianza del workspace o aprobación del workflow. */
export function LaunchBlockerNotice({ view, compact }: { view: RunView | undefined; compact?: boolean }) {
  const blocker = useLaunchBlocker(view);
  const toast = useToast();
  const [opening, setOpening] = useState(false);
  if (!blocker) return null;

  if (blocker.kind === "workflowReview") {
    const wf = blocker.workflow ?? "the workflow";
    return (
      <div className={`lb ${compact ? "lb-compact" : ""}`} role="alert">
        <span className="dot dot-lg" aria-hidden />
        <div className="lb-body">
          <span className="lb-title">Workflow not approved</span>
          <span className="lb-text">
            Claude Code asks to review new or changed workflows before running them, and a background session can't
            ask. Approve <span className="mono">{wf}</span> in <span className="mono">/workflows</span> in any Claude
            session, then run again.
          </span>
        </div>
      </div>
    );
  }

  const dir = blocker.dir;
  const open = async () => {
    if (!dir) return;
    setOpening(true);
    try {
      await openTerminalAt(dir, true);
      toast("Opened Terminal", `Accept the trust dialog in ${basename(dir)}, then run again.`);
    } catch (e) {
      toast("Couldn't open Terminal", String(e), "danger");
    } finally {
      setOpening(false);
    }
  };
  return (
    <div className={`lb ${compact ? "lb-compact" : ""}`} role="alert">
      <span className="dot dot-lg" aria-hidden />
      <div className="lb-body">
        <span className="lb-title">Workspace not trusted</span>
        <span className="lb-text">
          {blocker.home ? (
            "Claude Code never saves trust for your home directory. Map a project folder instead."
          ) : (
            <>
              Open Claude once in <span className="mono">{dir ?? "the repository"}</span> and accept the trust dialog,
              then run again.
            </>
          )}
        </span>
      </div>
      {dir && !blocker.home && (
        <button type="button" className="btn btn-sm" disabled={opening} onClick={() => void open()}>
          {opening ? "Opening…" : "Open in Terminal"}
        </button>
      )}
    </div>
  );
}
