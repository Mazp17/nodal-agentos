// In-app updates: a check on launch and from "Check for Updates…" in the app menu. A newer
// version is only installed when the user picks Update; nothing happens silently.

import { useCallback, useEffect, useRef } from "react";
import { check } from "@tauri-apps/plugin-updater";
import { onCheckForUpdates, restartApp, updatesEnabled } from "../../domain/api";
import { useConfirm } from "../../ui/ConfirmDialog";
import { SafeMarkdown } from "../../ui/Markdown";
import { useToast } from "../../ui/Toasts";
import "./updates.css";

export function useUpdates() {
  const ask = useConfirm();
  const toast = useToast();
  const busy = useRef(false);

  const run = useCallback(
    async (manual: boolean) => {
      if (busy.current) return;
      busy.current = true;
      let installing = false;
      try {
        const update = await check();
        if (!update) {
          if (manual) toast("You're up to date", undefined, "ok");
          return;
        }
        const ok = await ask({
          title: `Nodal ${update.version} is available`,
          body: update.body?.trim() ? <SafeMarkdown text={update.body} className="update-notes" /> : `You have ${update.currentVersion}.`,
          confirmLabel: "Update",
          cancelLabel: "Later",
          destructive: false,
        });
        if (!ok) return;
        installing = true;
        toast(`Downloading Nodal ${update.version}…`, "Nodal restarts when it's installed.");
        await update.downloadAndInstall();
        await restartApp();
      } catch (e) {
        if (installing) toast("Couldn't install the update", String(e), "danger");
        else if (manual) toast("Couldn't check for updates", String(e), "danger");
        else console.error("updates", e);
      } finally {
        busy.current = false;
      }
    },
    [ask, toast],
  );

  useEffect(() => {
    let cancelled = false;
    updatesEnabled()
      .then((enabled) => {
        if (enabled && !cancelled) void run(false);
      })
      .catch((e: unknown) => console.error("updates_enabled", e));
    const unlisten = onCheckForUpdates(() => void run(true));
    return () => {
      cancelled = true;
      void unlisten.then((f) => f());
    };
  }, [run]);
}
