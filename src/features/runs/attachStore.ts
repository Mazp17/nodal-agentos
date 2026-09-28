// Which run the "Attached session" drawer shows. A tiny external store (not props) so any
// `useRunActions().attach` caller can open it without threading a callback through every view;
// `AppShell` renders the drawer and clears it on navigation.

import { useSyncExternalStore } from "react";

export interface AttachTarget {
  /** Nodal run id (`Run.id`), to show its status and title. */
  runId: string;
  /** Id passed to `claude attach` (`Run.claudeRunId`, or the live session's). */
  claudeId: string;
  /** Working directory of the session, if known. */
  cwd: string | null;
}

let current: AttachTarget | null = null;
const listeners = new Set<() => void>();

function set(next: AttachTarget | null) {
  if (next === current) return;
  current = next;
  for (const l of listeners) l();
}

/** Opens the drawer on `target` (replacing any session shown). */
export const openAttachedSession = (target: AttachTarget) => set(target);

/** Closes the drawer: only detaches, the run keeps going. */
export const closeAttachedSession = () => set(null);

const subscribe = (l: () => void) => {
  listeners.add(l);
  return () => {
    listeners.delete(l);
  };
};
const snapshot = () => current;

export function useAttachTarget(): AttachTarget | null {
  return useSyncExternalStore(subscribe, snapshot);
}
