// Unsaved edits of open file tabs, kept outside the components: a file tab unmounts when
// another tab is picked, and its edits must still be there when it comes back.

import { useSyncExternalStore } from "react";

export interface Draft {
  text: string;
  /** Version of the file on disk the edits started from (`RepoFile.version`). */
  version: string;
}

const drafts = new Map<string, Draft>();
let dirty: ReadonlySet<string> = new Set();
const listeners = new Set<() => void>();

function emit() {
  dirty = new Set(drafts.keys());
  for (const l of listeners) l();
}

export const getDraft = (tab: string): Draft | undefined => drafts.get(tab);

export function setDraft(tab: string, draft: Draft | null) {
  if (draft) drafts.set(tab, draft);
  else drafts.delete(tab);
  emit();
}

/** Tabs with unsaved edits (their tab shows a dot; closing asks first). */
export function useDirtyTabs(): ReadonlySet<string> {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => dirty,
  );
}
