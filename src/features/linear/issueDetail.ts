import { useCallback, useEffect, useState } from "react";
import { linearApi, toLinearError, type IssueDetail, type LinearError } from "./api";

/**
 * Caché del detalle por issue mientras el panel está abierto (App la vacía al cerrarlo).
 * Guarda también la petición en vuelo para no duplicarla si se navega ida y vuelta
 * entre sub-issues antes de que responda.
 */
export class IssueDetailCache {
  private done = new Map<string, IssueDetail>();
  private inflight = new Map<string, Promise<IssueDetail>>();

  peek(id: string): IssueDetail | undefined {
    return this.done.get(id);
  }

  load(id: string): Promise<IssueDetail> {
    const hit = this.done.get(id);
    if (hit) return Promise.resolve(hit);
    let p = this.inflight.get(id);
    if (!p) {
      p = linearApi.issueDetail(id).then(
        (d) => {
          if (this.inflight.get(id) === p) {
            this.done.set(id, d);
            this.inflight.delete(id);
          }
          return d;
        },
        (err) => {
          // Los errores no se cachean: "Retry" vuelve a pedir.
          if (this.inflight.get(id) === p) this.inflight.delete(id);
          throw err;
        },
      );
      this.inflight.set(id, p);
    }
    return p;
  }

  clear() {
    this.done.clear();
    this.inflight.clear();
  }
}

export type DetailState =
  | { status: "loading" }
  | { status: "ready"; detail: IssueDetail }
  | { status: "error"; error: LinearError };

export function useIssueDetail(issueId: string, cache: IssueDetailCache): [DetailState, () => void] {
  const [state, setState] = useState<{ id: string; s: DetailState }>(() => {
    const hit = cache.peek(issueId);
    return { id: issueId, s: hit ? { status: "ready", detail: hit } : { status: "loading" } };
  });
  const [attempt, setAttempt] = useState(0);

  useEffect(() => {
    let alive = true;
    const hit = cache.peek(issueId);
    setState({ id: issueId, s: hit ? { status: "ready", detail: hit } : { status: "loading" } });
    if (!hit) {
      cache.load(issueId).then(
        (detail) => alive && setState({ id: issueId, s: { status: "ready", detail } }),
        (err) => alive && setState({ id: issueId, s: { status: "error", error: toLinearError(err) } }),
      );
    }
    return () => {
      alive = false;
    };
  }, [issueId, cache, attempt]);

  const retry = useCallback(() => setAttempt((n) => n + 1), []);
  // Mientras el efecto no corrió para un id nuevo, no mostrar datos del anterior.
  return [state.id === issueId ? state.s : { status: "loading" }, retry];
}
