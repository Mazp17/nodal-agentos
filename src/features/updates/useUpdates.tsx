// In-app updates: a check on launch, once a day while the app stays open, and on demand from
// "Check for Updates…" in the app menu or Settings → Updates. A newer version is only installed
// when the user picks Update; nothing happens silently.

import { useCallback, useEffect, useRef, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { check } from "@tauri-apps/plugin-updater";
import { onCheckForUpdates, restartApp, updatesEnabled } from "../../domain/api";
import { useConfirm } from "../../ui/ConfirmDialog";
import { SafeMarkdown } from "../../ui/Markdown";
import { useToast } from "../../ui/Toasts";
import "./updates.css";

const DAY_MS = 24 * 60 * 60 * 1000;
/** How often the open app looks at whether a day has passed since the last check. */
const TICK_MS = 60 * 60 * 1000;

/**
 * - `launch`: on start; offers the update, errors only go to the console.
 * - `manual`: menu or Settings; offers the update and always says how it went.
 * - `daily`: while the app is open; no dialog, a toast the first time a version shows up.
 */
type Mode = "launch" | "manual" | "daily";

export interface Updates {
  /** false in debug builds ("Nodal Dev"), null until known. */
  enabled: boolean | null;
  /** Installed version; null until it's read. */
  version: string | null;
  /** Newer version found by the last check, also after "Later"; null if none. */
  available: string | null;
  checking: boolean;
  /** Downloading and installing; the app restarts when it's done. */
  installing: boolean;
  /** When the last check finished (with or without an update); null if none has. */
  lastChecked: Date | null;
  /** Check again and offer the update, as the menu item does. */
  check: () => void;
}

export function useUpdates(): Updates {
  const ask = useConfirm();
  const toast = useToast();
  const busy = useRef(false);
  const lastCheckAt = useRef(0);
  const announced = useRef<string | null>(null);
  const [enabled, setEnabled] = useState<boolean | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [available, setAvailable] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [lastChecked, setLastChecked] = useState<Date | null>(null);

  const run = useCallback(
    async (mode: Mode) => {
      if (busy.current) {
        if (mode === "manual") toast("Already checking for updates");
        return;
      }
      busy.current = true;
      setChecking(true);
      let installStarted = false;
      try {
        const update = await check();
        lastCheckAt.current = Date.now();
        setLastChecked(new Date(lastCheckAt.current));
        setAvailable(update?.version ?? null);
        if (!update) {
          if (mode === "manual") toast("You're up to date", undefined, "ok");
          return;
        }
        if (mode === "daily") {
          if (announced.current !== update.version) {
            announced.current = update.version;
            toast(`Nodal ${update.version} is available`, "Install it from the sidebar or Settings → Updates.", "accent");
          }
          return;
        }
        announced.current = update.version;
        const ok = await ask({
          title: `Nodal ${update.version} is available`,
          body: update.body?.trim() ? <SafeMarkdown text={update.body} className="update-notes" /> : `You have ${update.currentVersion}.`,
          confirmLabel: "Update",
          cancelLabel: "Later",
          destructive: false,
        });
        if (!ok) return;
        installStarted = true;
        setInstalling(true);
        toast(`Downloading Nodal ${update.version}…`, "Nodal restarts when it's installed.");
        await update.downloadAndInstall();
        await restartApp();
      } catch (e) {
        if (installStarted) toast("Couldn't install the update", String(e), "danger");
        else if (mode === "manual") toast("Couldn't check for updates", String(e), "danger");
        else console.error("updates", e);
      } finally {
        busy.current = false;
        setChecking(false);
        setInstalling(false);
      }
    },
    [ask, toast],
  );

  useEffect(() => {
    let cancelled = false;
    let timer: ReturnType<typeof setInterval> | undefined;
    getVersion()
      .then((v) => {
        if (!cancelled) setVersion(v);
      })
      .catch((e: unknown) => console.error("getVersion", e));
    updatesEnabled()
      .then((on) => {
        if (cancelled) return;
        setEnabled(on);
        if (!on) return;
        void run("launch");
        // Hourly tick instead of a 24 h timer: timers drift while the Mac sleeps.
        timer = setInterval(() => {
          if (Date.now() - lastCheckAt.current >= DAY_MS) void run("daily");
        }, TICK_MS);
      })
      .catch((e: unknown) => console.error("updates_enabled", e));
    const unlisten = onCheckForUpdates(() => void run("manual"));
    return () => {
      cancelled = true;
      clearInterval(timer);
      void unlisten.then((f) => f());
    };
  }, [run]);

  const checkNow = useCallback(() => void run("manual"), [run]);
  return { enabled, version, available, checking, installing, lastChecked, check: checkNow };
}
