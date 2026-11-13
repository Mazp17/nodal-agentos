// Theme preference (`nodal.theme`): system | dark | light. The resolved theme lives in
// `<html data-theme>`, which tokens.css keys off; "system" follows macOS live.

import { useSyncExternalStore } from "react";
import { readPref, writePref } from "./storage";

export type ThemePref = "system" | "dark" | "light";
export type Theme = "dark" | "light";

const media = window.matchMedia("(prefers-color-scheme: light)");
const listeners = new Set<() => void>();
let pref: ThemePref = readThemePref();

function readThemePref(): ThemePref {
  const v = readPref("theme");
  return v === "dark" || v === "light" ? v : "system";
}

function apply() {
  const theme: Theme = pref === "system" ? (media.matches ? "light" : "dark") : pref;
  document.documentElement.dataset.theme = theme;
}

/** Applies the saved theme and follows the OS. Call once, before the first render (no flash). */
export function installTheme() {
  apply();
  media.addEventListener("change", () => {
    if (pref === "system") apply();
  });
}

export function setThemePref(next: ThemePref) {
  pref = next;
  writePref("theme", next);
  apply();
  for (const l of listeners) l();
}

export function useThemePref(): ThemePref {
  return useSyncExternalStore(
    (l) => {
      listeners.add(l);
      return () => listeners.delete(l);
    },
    () => pref,
  );
}

/** Calls `cb` whenever the resolved theme changes (pref or OS). Returns the unsubscribe. */
export function onThemeChange(cb: () => void): () => void {
  const obs = new MutationObserver(cb);
  obs.observe(document.documentElement, { attributes: true, attributeFilter: ["data-theme"] });
  return () => obs.disconnect();
}

const currentTheme = () => (document.documentElement.dataset.theme === "light" ? "light" : "dark");

export function useTheme(): Theme {
  return useSyncExternalStore(onThemeChange, currentTheme);
}
