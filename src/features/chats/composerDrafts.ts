// Unsent composer text per chat tab, kept outside `ChatView`: the view unmounts when another
// tab or view is picked, and the text must still be there when it comes back. In memory only.

import { useSyncExternalStore } from "react";
import type { ImageInput } from "../../domain/api";

const drafts = new Map<string, string>();
const images = new Map<string, ImageInput[]>();
const NO_IMAGES: ImageInput[] = [];
const listeners = new Set<() => void>();

export const getComposerDraft = (key: string): string => drafts.get(key) ?? "";

export function setComposerDraft(key: string, text: string) {
  if (text) drafts.set(key, text);
  else drafts.delete(key);
  for (const l of listeners) l();
}

export const getComposerImages = (key: string): ImageInput[] => images.get(key) ?? NO_IMAGES;

export function setComposerImages(key: string, list: ImageInput[]) {
  if (list.length) images.set(key, list);
  else images.delete(key);
  for (const l of listeners) l();
}

export function useComposerImages(key: string): ImageInput[] {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => getComposerImages(key),
  );
}

/** Key: the chat's id, or `new:<projectId>` for a chat not created yet. */
export function useComposerDraft(key: string): string {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => getComposerDraft(key),
  );
}
