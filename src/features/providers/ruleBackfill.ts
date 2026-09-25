// Project rules (provider project → repo): save and backfill.
// Used by Project settings → Repos (per-repo picker) and → Sources (link rules).

import { useCallback, useState } from "react";
import { importRule, listSourceLinks, previewRuleImport, updateSourceLink, type ImportResult } from "../../domain/api";
import { errorText, invalidateProviders } from "../../domain/hooks/providers";
import type { RepoRule, ScopeRef, SourceLink } from "../../domain/types";
import { useConfirm } from "../../ui/ConfirmDialog";
import { useToast } from "../../ui/Toasts";
import { plural } from "./meta";

/** New rule: the backend assigns `id` and `createdAt` on save. */
export function newProjectRule(project: ScopeRef, repoId: string): RepoRule {
  return { id: "", kind: "project", value: project.id, name: project.name, repoId, createdAt: 0 };
}

/** Newly created/changed project rule to offer importing for. */
export interface BackfillTarget {
  projectId: string;
  projectName: string;
  repoId: string;
  repoName: string;
}

/** Rule change on a link: `update` gets the list freshly read from the backend, not the rendered one. */
export interface RuleEdit {
  link: SourceLink;
  update: (rules: RepoRule[]) => RepoRule[];
}

export interface RuleSaver {
  busy: boolean;
  /** What it's doing now ("Checking Linear…", "Importing 12 issues…"); null if nothing. */
  phase: string | null;
  /** Result of the last backfill/sync ("Imported 3 issues"), shown next to the control. */
  last: { ruleId: string; text: string; tone: "ok" | "warn" | "danger"; at: number } | null;
  /** A `resync` is in progress. */
  syncing: boolean;
  /**
   * Applies `edits` in order. If one fails, undoes the previous ones (re-saves their prior list).
   * With `backfill`, then offers to import what already exists in that project
   * (preview → confirmation → `importRule` → toast). Returns false if not saved (already reported).
   */
  save: (edits: RuleEdit | RuleEdit[], backfill?: BackfillTarget) => Promise<boolean>;
  /** Re-imports what already exists in a saved rule's project, without confirmation. */
  resync: (link: SourceLink, rule: RepoRule, repoName: string) => Promise<void>;
}

/*
 * All rule writes (and their confirmations) go through a single queue: two cards or
 * two quick clicks don't clobber each other, and each starts from the saved list, not the one left
 * rendered before `invalidateProviders("links")` reloads.
 */
let queue: Promise<unknown> = Promise.resolve();

function enqueue<T>(job: () => Promise<T>): Promise<T> {
  const run = queue.then(job, job);
  queue = run.catch(() => undefined);
  return run;
}

async function freshRules(link: SourceLink): Promise<RepoRule[]> {
  const links = await listSourceLinks(link.projectId);
  const found = links.find((l) => l.id === link.id);
  if (!found) throw new Error("This source was disconnected.");
  return found.repoRules;
}

export function useRuleBackfill(): RuleSaver {
  const ask = useConfirm();
  const toast = useToast();
  const [pending, setPending] = useState(0);
  const [phase, setPhase] = useState<string | null>(null);
  const [last, setLast] = useState<RuleSaver["last"]>(null);
  const [syncing, setSyncing] = useState(0);
  const report = useCallback(
    (ruleId: string, text: string, tone: "ok" | "warn" | "danger") => setLast({ ruleId, text, tone, at: Date.now() }),
    [],
  );

  /** `importRule` with a visible phase, toast and inline result. */
  const runImport = useCallback(
    async (linkId: string, ruleId: string, repoName: string, count?: number) => {
      setPhase(count ? `Importing ${plural(count, "issue")}…` : "Importing issues…");
      const [title, body, tone] = importToast(await importRule(linkId, ruleId), repoName);
      toast(title, body, tone);
      report(ruleId, title, tone);
    },
    [report, toast],
  );

  const runBackfill = useCallback(
    async (savedLinks: SourceLink[], t: BackfillTarget) => {
      const match = (r: RepoRule) => r.kind === "project" && r.value === t.projectId && r.repoId === t.repoId;
      const saved = savedLinks.find((l) => l.repoRules.some(match));
      const rule = saved?.repoRules.find(match);
      if (!saved || !rule) return;
      try {
        setPhase(`Checking ${t.projectName} in Linear…`);
        const p = await previewRuleImport(saved.id, rule.id);
        setPhase(null);
        const elsewhere = p.inOtherRepos > 0 ? `${p.inOtherRepos} already imported elsewhere stay where they are` : null;
        if (p.count === 0) {
          toast("Rule saved. No issues to import", elsewhere ? `${elsewhere}.` : undefined, "ok");
          report(rule.id, `No issues to import${elsewhere ? ` (${elsewhere})` : ""}`, "ok");
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
          report(rule.id, `Skipped ${plural(p.count, "existing issue")}; only new ones will be imported`, "ok");
          return;
        }
        await runImport(saved.id, rule.id, t.repoName, p.count);
      } catch (err) {
        toast("Couldn't import issues", `${errorText(err)}\nThe rule is saved; only new issues will be imported.`, "danger");
        report(rule.id, `Import failed: ${errorText(err)}`, "danger");
      } finally {
        setPhase(null);
      }
    },
    [ask, report, runImport, toast],
  );

  const save = useCallback(
    (input: RuleEdit | RuleEdit[], backfill?: BackfillTarget) => {
      const edits = Array.isArray(input) ? input : [input];
      setPending((n) => n + 1);
      return enqueue(async () => {
        setPhase("Saving…");
        /** Prior list of each already-saved edit, for undoing. */
        const done: { link: SourceLink; before: RepoRule[] }[] = [];
        const saved: SourceLink[] = [];
        try {
          for (const e of edits) {
            const before = await freshRules(e.link);
            saved.push(await updateSourceLink(e.link.id, { repoRules: e.update(before) }));
            done.push({ link: e.link, before });
          }
        } catch (err) {
          const rollback = await undo(done);
          toast("Couldn't save the rule", [errorText(err), rollback].filter(Boolean).join("\n"), "danger");
          return false;
        } finally {
          setPhase(null);
          invalidateProviders("links");
        }
        if (backfill) await runBackfill(saved, backfill);
        return true;
      }).finally(() => setPending((n) => n - 1));
    },
    [runBackfill, toast],
  );

  const resync = useCallback(
    (link: SourceLink, rule: RepoRule, repoName: string) => {
      setPending((n) => n + 1);
      setSyncing((n) => n + 1);
      return enqueue(async () => {
        try {
          await runImport(link.id, rule.id, repoName);
        } catch (err) {
          toast("Couldn't sync issues", errorText(err), "danger");
          report(rule.id, `Sync failed: ${errorText(err)}`, "danger");
        } finally {
          setPhase(null);
        }
      }).finally(() => {
        setPending((n) => n - 1);
        setSyncing((n) => n - 1);
      });
    },
    [report, runImport, toast],
  );

  return { busy: pending > 0, syncing: syncing > 0, phase, last, save, resync };
}

/** Re-saves the prior lists (in reverse order). Returns what couldn't be restored, if anything. */
async function undo(done: { link: SourceLink; before: RepoRule[] }[]): Promise<string | null> {
  const lost: string[] = [];
  for (const d of [...done].reverse()) {
    try {
      await updateSourceLink(d.link.id, { repoRules: d.before });
    } catch {
      const now = await freshRules(d.link).catch(() => [] as RepoRule[]);
      const ids = new Set(now.map((r) => `${r.kind}:${r.value}:${r.repoId}`));
      for (const r of d.before) if (!ids.has(`${r.kind}:${r.value}:${r.repoId}`)) lost.push(`${r.kind}: ${r.name || r.value}`);
    }
  }
  if (!done.length) return null;
  return lost.length
    ? `These rules were removed and couldn't be restored: ${lost.join(", ")}.`
    : "Earlier changes were undone.";
}

function importToast(r: ImportResult, repoName: string): [string, string | undefined, "ok" | "warn"] {
  const title = r.imported.length ? `Imported ${plural(r.imported.length, "issue")} into ${repoName}` : "No issues imported";
  if (!r.skipped.length) return [title, undefined, "ok"];
  // Groups by reason: "Already imported in another repo (×3)".
  const byReason = new Map<string, number>();
  for (const s of r.skipped) byReason.set(s.reason, (byReason.get(s.reason) ?? 0) + 1);
  const lines = [...byReason].map(([reason, n]) => (n > 1 ? `${reason} (×${n})` : reason));
  return [title, `${plural(r.skipped.length, "issue")} skipped:\n${lines.join("\n")}`, "warn"];
}
