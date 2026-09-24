// Reglas de proyecto (proyecto del proveedor → repo): guardar y backfill.
// Lo usan Project settings → Repos (selector por repo) y → Sources (reglas del link).

import { useCallback, useState } from "react";
import { importRule, previewRuleImport, updateSourceLink, type ImportResult } from "../../domain/api";
import { errorText, invalidateProviders } from "../../domain/hooks/providers";
import type { RepoRule, ScopeRef, SourceLink } from "../../domain/types";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { plural } from "./meta";

/** Regla nueva: el backend le asigna `id` y `createdAt` al guardar. */
export function newProjectRule(project: ScopeRef, repoId: string): RepoRule {
  return { id: "", kind: "project", value: project.id, name: project.name, repoId, createdAt: 0 };
}

/** Regla de proyecto recién creada/cambiada que hay que ofrecer importar. */
export interface BackfillTarget {
  projectId: string;
  projectName: string;
  repoId: string;
  repoName: string;
}

export interface RuleSaver {
  busy: boolean;
  /**
   * Guarda la lista completa `rules` en `link`. Con `backfill`, después ofrece importar lo que
   * ya existe en ese proyecto (preview → confirmación → `importRule` → toast).
   * Devuelve false si no se pudo guardar (ya avisó con un toast).
   */
  save: (link: SourceLink, rules: RepoRule[], backfill?: BackfillTarget) => Promise<boolean>;
}

export function useRuleBackfill(): RuleSaver {
  const ask = useConfirm();
  const toast = useToast();
  const [busy, setBusy] = useState(false);

  const runBackfill = useCallback(
    async (saved: SourceLink, t: BackfillTarget) => {
      const rule = saved.repoRules.find((r) => r.kind === "project" && r.value === t.projectId && r.repoId === t.repoId);
      if (!rule) return;
      try {
        const p = await previewRuleImport(saved.id, rule.id);
        const elsewhere = p.inOtherRepos > 0 ? `${p.inOtherRepos} already imported elsewhere stay where they are` : null;
        if (p.count === 0) {
          toast("Rule saved. No issues to import", elsewhere ? `${elsewhere}.` : undefined, "ok");
          return;
        }
        const ok = await ask({
          title: `Import ${plural(p.count, "issue")} from ${t.projectName} into ${t.repoName}?${elsewhere ? ` (${elsewhere})` : ""}`,
          body: [
            p.alreadyImported > 0 ? `${plural(p.alreadyImported, "issue")} already in ${t.repoName}.` : null,
            "Either way the rule is saved and new issues in this project will be imported automatically.",
          ]
            .filter(Boolean)
            .join(" "),
          confirmLabel: "Import",
          cancelLabel: "Not now",
          destructive: false,
        });
        if (!ok) {
          toast("Rule saved", "Only new issues will be imported.", "info");
          return;
        }
        toast(...importToast(await importRule(saved.id, rule.id), t.repoName));
      } catch (err) {
        toast("Couldn't import issues", `${errorText(err)}\nThe rule is saved; only new issues will be imported.`, "danger");
      }
    },
    [ask, toast],
  );

  const save = useCallback(
    async (link: SourceLink, rules: RepoRule[], backfill?: BackfillTarget) => {
      setBusy(true);
      try {
        let saved: SourceLink;
        try {
          saved = await updateSourceLink(link.id, { repoRules: rules });
        } catch (err) {
          toast("Couldn't save the rule", errorText(err), "danger");
          return false;
        } finally {
          invalidateProviders("links");
        }
        if (backfill) await runBackfill(saved, backfill);
        return true;
      } finally {
        setBusy(false);
      }
    },
    [runBackfill, toast],
  );

  return { busy, save };
}

function importToast(r: ImportResult, repoName: string): [string, string | undefined, "ok" | "warn"] {
  const title = r.imported.length ? `Imported ${plural(r.imported.length, "issue")} into ${repoName}` : "No issues imported";
  if (!r.skipped.length) return [title, undefined, "ok"];
  // Agrupa por motivo: "Already imported in another repo (×3)".
  const byReason = new Map<string, number>();
  for (const s of r.skipped) byReason.set(s.reason, (byReason.get(s.reason) ?? 0) + 1);
  const lines = [...byReason].map(([reason, n]) => (n > 1 ? `${reason} (×${n})` : reason));
  return [title, `${plural(r.skipped.length, "issue")} skipped:\n${lines.join("\n")}`, "warn"];
}
