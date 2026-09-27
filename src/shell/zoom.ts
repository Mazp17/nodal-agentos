// App zoom with ⌘+ / ⌘- / ⌘0 (Ctrl on other platforms), remembered across launches.

import { getCurrentWebview } from "@tauri-apps/api/webview";
import { readPref, writePref } from "./storage";

// Same steps as a browser, so each press feels familiar.
const LEVELS = [0.5, 0.67, 0.75, 0.8, 0.9, 1, 1.1, 1.25, 1.5, 1.75, 2];
const DEFAULT = LEVELS.indexOf(1);

function readLevel(): number {
  const raw = readPref("zoom");
  if (raw === null) return DEFAULT;
  const i = Number(raw);
  return Number.isInteger(i) && i >= 0 && i < LEVELS.length ? i : DEFAULT;
}

function apply(i: number) {
  getCurrentWebview()
    .setZoom(LEVELS[i]!)
    .catch(() => {
      /* outside Tauri (plain `vite`) there is no webview to zoom */
    });
}

/** Restores the saved zoom and listens for the shortcuts. Call once, before the first render. */
export function installZoomShortcuts() {
  let level = readLevel();
  if (level !== DEFAULT) apply(level);

  window.addEventListener("keydown", (e) => {
    if (!(e.metaKey || e.ctrlKey) || e.altKey || e.isComposing) return;
    let next: number;
    if (e.key === "+" || e.key === "=" || e.code === "NumpadAdd") next = Math.min(level + 1, LEVELS.length - 1);
    else if (e.key === "-" || e.code === "NumpadSubtract") next = Math.max(level - 1, 0);
    else if (!e.shiftKey && (e.key === "0" || e.code === "Digit0" || e.code === "Numpad0")) next = DEFAULT;
    else return;
    e.preventDefault();
    if (next === level) return;
    level = next;
    writePref("zoom", level === DEFAULT ? null : String(level));
    apply(level);
  });
}
