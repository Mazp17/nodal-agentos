// "Import data from a previous version…": folder picked with the native picker, copied to
// `legacy-backup-<ts>/`, and imported idempotently (`import_legacy_data`).

import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { importLegacyData, type LegacyImportReport } from "../../domain/api";
import { invalidate } from "../../domain/hooks/store";
import { useToast } from "../../ui/Toasts";

export function useLegacyImport() {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<LegacyImportReport | null>(null);
  const [error, setError] = useState<string | null>(null);

  const run = async (): Promise<LegacyImportReport | null> => {
    let picked: string | string[] | null;
    try {
      picked = await open({ directory: true, multiple: false, title: "Choose the data folder of a previous version" });
    } catch (e) {
      toast("Couldn't open the folder picker", String(e), "danger");
      return null;
    }
    const folder = typeof picked === "string" ? picked : null;
    if (!folder) return null;
    setBusy(true);
    setError(null);
    try {
      const r = await importLegacyData(folder);
      setReport(r);
      await invalidate("projects", "repos", "tasks", "runs", "settings");
      toast("Data imported", summarize(r), "ok");
      return r;
    } catch (e) {
      setError(String(e));
      toast("Import failed", String(e), "danger");
      return null;
    } finally {
      setBusy(false);
    }
  };

  return { run, busy, report, error };
}

export function summarize(r: LegacyImportReport): string {
  const n = (k: number, one: string) => `${k} ${one}${k === 1 ? "" : "s"}`;
  const parts = [n(r.projects, "project"), n(r.repos, "repo"), n(r.tasks, "task"), n(r.runs, "run")];
  if (r.alreadyImported) parts.push(`${r.alreadyImported} already imported`);
  if (r.skipped.length) parts.push(`${r.skipped.length} skipped`);
  return parts.join(" · ");
}
