import { useId, useState } from "react";
import { useRepos } from "../../domain/hooks/store";
import { importTasks, type ImportResult, type ImportableItem } from "../../domain/api";
import type { Repo, SourceLink } from "../../domain/types";
import {
  errorText,
  useDebounced,
  useImportable,
  useProviderStatus,
  useSourceLinks,
} from "../../domain/hooks/providers";
import { useFocusTrap } from "../../ui/useFocusTrap";
import { useToast } from "../../ui/Toasts";
import { Segmented } from "./parts";
import { ExtStateLabel } from "./StateMapEditor";
import { plural, providerName } from "./meta";

export interface ImportDialogProps {
  projectId: string;
  /** Nombre para el título ("Import into Payments"); sin él, "Import tasks". */
  projectName?: string;
  onClose: () => void;
  onImported: (result: ImportResult) => void;
  /** Sin fuentes conectadas: botón para ir a Project settings → Sources. */
  onOpenSources?: () => void;
}

/** Diálogo de importación: elegir ítems del proveedor y el repo de cada uno. */
export function ImportDialog({ projectId, projectName, onClose, onImported, onOpenSources }: ImportDialogProps) {
  const ref = useFocusTrap<HTMLDivElement>(onClose);
  const titleId = useId();
  const links = useSourceLinks(projectId);
  const repos = useRepos(projectId);
  const [linkId, setLinkId] = useState<string | null>(null);
  const link = links.data?.find((l) => l.id === linkId) ?? links.data?.[0] ?? null;

  return (
    <>
      <div className="pv-scrim" onClick={onClose} aria-hidden />
      <div ref={ref} className="pv-dialog" role="dialog" aria-modal="true" aria-labelledby={titleId} tabIndex={-1}>
        <header className="pv-dialog-head">
          <h2 id={titleId} className="pv-dialog-title">
            {projectName ? `Import into ${projectName}` : "Import tasks"}
          </h2>
          <button type="button" className="icon-btn" aria-label="Close" onClick={onClose}>
            ✕
          </button>
        </header>

        {links.error ? (
          <div className="pv-dialog-body">
            <div className="pv-alert pv-alert-danger" role="alert">
              <span className="pv-alert-text">Could not load sources. {links.error}</span>
              <button type="button" className="btn btn-sm" onClick={links.reload}>
                Retry
              </button>
            </div>
          </div>
        ) : !links.data ? (
          <div className="pv-dialog-body">
            <span className="pv-hint">Loading sources…</span>
          </div>
        ) : !link ? (
          <div className="pv-dialog-body">
            <div className="pv-empty pv-empty-dashed">
              No sources connected to this project. Connect a task manager in Project settings → Sources.
            </div>
            {onOpenSources && (
              <div className="pv-actions pv-actions-end">
                <button type="button" className="btn btn-primary btn-sm" onClick={onOpenSources}>
                  Open Sources
                </button>
              </div>
            )}
          </div>
        ) : (
          <ImportBody
            key={link.id}
            projectId={projectId}
            link={link}
            links={links.data}
            repos={repos.data ?? []}
            onPickLink={setLinkId}
            onClose={onClose}
            onImported={onImported}
          />
        )}
      </div>
    </>
  );
}

function ImportBody({
  projectId,
  link,
  links,
  repos,
  onPickLink,
  onClose,
  onImported,
}: {
  projectId: string;
  link: SourceLink;
  links: SourceLink[];
  repos: Repo[];
  onPickLink: (id: string) => void;
  onClose: () => void;
  onImported: (r: ImportResult) => void;
}) {
  const toast = useToast();
  const prov = providerName(link.provider);
  const st = useProviderStatus(link.provider);
  const keyOk = st.connection === "connected";
  const [query, setQuery] = useState("");
  const q = useDebounced(query, 300);
  const items = useImportable(keyOk ? link.id : null, q);
  /** Seleccionados, con el repo elegido (o `null` = el sugerido). */
  const [sel, setSel] = useState<Map<string, ImportableItem>>(new Map());
  const [repoOverride, setRepoOverride] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const repoFor = (it: ImportableItem) => repoOverride[it.externalId] ?? it.suggestedRepoId ?? "";
  const selected = [...sel.values()];
  const missingRepo = selected.filter((it) => !repoFor(it));
  const canImport = keyOk && selected.length > 0 && missingRepo.length === 0 && !busy;

  const toggle = (it: ImportableItem) =>
    setSel((m) => {
      const next = new Map(m);
      if (next.has(it.externalId)) next.delete(it.externalId);
      else next.set(it.externalId, it);
      return next;
    });

  const run = async () => {
    if (!canImport) return;
    setBusy(true);
    setError(null);
    try {
      const r = await importTasks(
        projectId,
        link.id,
        selected.map((it) => ({ externalId: it.externalId, repoId: repoFor(it) })),
      );
      if (r.imported.length) {
        toast(`Imported ${plural(r.imported.length, "task")}`, `From ${prov} · ${link.scope.name}`, "ok");
      }
      onImported(r);
      if (!r.skipped.length) onClose();
      else {
        // Quedan seleccionados solo los que fallaron, para reintentar o cambiar de repo.
        const failed = new Set(r.skipped.map((s) => s.externalId));
        const names = new Map(selected.map((it) => [it.externalId, it.identifier]));
        setError(r.skipped.map((s) => `${names.get(s.externalId) ?? s.externalId}: ${s.reason}`).join("\n"));
        setSel((m) => new Map([...m].filter(([id]) => failed.has(id))));
        items.reload();
      }
    } catch (err) {
      setError(errorText(err));
    } finally {
      setBusy(false);
    }
  };

  const list = items.data ?? [];
  const selectable = list.filter((it) => !it.taskId);
  const allOn = selectable.length > 0 && selectable.every((it) => sel.has(it.externalId));

  return (
    <>
      <div className="pv-dialog-body">
        {links.length > 1 && (
          <div className="pv-field">
            <span className="pv-field-label">Source</span>
            <div className="pv-field-body">
              <Segmented
                value={link.id}
                label="Source"
                options={links.map((l) => ({ value: l.id, label: `${providerName(l.provider)} · ${l.scope.name}` }))}
                onChange={onPickLink}
              />
            </div>
          </div>
        )}
        {!keyOk && st.connection !== "loading" && (
          <div className="pv-alert pv-alert-danger" role="alert">
            {st.status?.hasKey
              ? `${prov} is not reachable with the saved key${st.error ? `: ${st.error}` : "."}`
              : `${prov} is disconnected. Add a key in Settings → Integrations.`}
          </div>
        )}
        {link.stateMap.confirmedAt === null && (
          <div className="pv-alert pv-alert-warn" role="status">
            Mapping pending: you can import, but Nodal won't push status changes to {prov} until you confirm the
            mapping in Project settings → Sources.
          </div>
        )}
        <input
          type="search"
          className="input pv-search"
          placeholder="Search open issues"
          aria-label="Search open issues"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          autoFocus
        />

        <div className="pv-imp-table" role="table" aria-label="Issues" aria-busy={items.loading}>
          <div className="pv-imp-row pv-imp-head" role="row">
            <span role="columnheader">
              <input
                type="checkbox"
                aria-label="Select all"
                checked={allOn}
                disabled={selectable.length === 0}
                onChange={() =>
                  setSel((m) => {
                    const next = new Map(m);
                    if (allOn) selectable.forEach((it) => next.delete(it.externalId));
                    else selectable.forEach((it) => next.set(it.externalId, it));
                    return next;
                  })
                }
              />
            </span>
            <span role="columnheader">Issue</span>
            <span role="columnheader">Title</span>
            <span role="columnheader">Repo</span>
          </div>
          {items.error && (
            <div className="pv-alert pv-alert-danger pv-imp-msg" role="alert">
              <span className="pv-alert-text">Could not load issues. {items.error}</span>
              <button type="button" className="btn btn-sm" onClick={items.reload}>
                Retry
              </button>
            </div>
          )}
          {!items.data && items.loading && <div className="pv-empty">Loading issues…</div>}
          {list.map((it) => (
            <ImportRow
              key={it.externalId}
              item={it}
              link={link}
              repos={repos}
              checked={sel.has(it.externalId)}
              repoId={repoFor(it)}
              overridden={it.externalId in repoOverride}
              onToggle={() => toggle(it)}
              onRepo={(id) => setRepoOverride((o) => ({ ...o, [it.externalId]: id }))}
            />
          ))}
          {items.data && list.length === 0 && (
            <div className="pv-empty">{q ? "No open issues match." : "No open issues in this scope."}</div>
          )}
        </div>
        {items.data && list.length >= 100 && (
          <span className="pv-hint">Showing the first 100 issues. Search to narrow the list.</span>
        )}
        {error && (
          <div className="pv-alert pv-alert-danger" role="alert">
            {error}
          </div>
        )}
      </div>
      <footer className="pv-dialog-foot">
        <span className="pv-hint pv-grow">
          {missingRepo.length > 0
            ? `Pick a repo for ${missingRepo.map((it) => it.identifier).join(", ")}.`
            : "Repo comes from the source's default and routing rules. Change it per issue."}
        </span>
        <button type="button" className="btn btn-ghost" onClick={onClose} disabled={busy}>
          Cancel
        </button>
        <button type="button" className="btn btn-primary" disabled={!canImport} onClick={() => void run()}>
          {busy ? "Importing…" : selected.length ? `Import ${plural(selected.length, "task")}` : "Import tasks"}
        </button>
      </footer>
    </>
  );
}

function ImportRow({
  item,
  link,
  repos,
  checked,
  repoId,
  overridden,
  onToggle,
  onRepo,
}: {
  item: ImportableItem;
  link: SourceLink;
  repos: Repo[];
  checked: boolean;
  repoId: string;
  overridden: boolean;
  onToggle: () => void;
  onRepo: (id: string) => void;
}) {
  const done = item.taskId !== null;
  const cbId = useId();
  const why = overridden || !repoId ? null : repoId === link.defaultRepoId && !ruleMatches(link, item) ? "default" : "rule";
  return (
    <div role="row" className={`pv-imp-row${done ? " is-done" : ""}${checked ? " is-on" : ""}`}>
      <span role="cell">
        <input id={cbId} type="checkbox" checked={checked} disabled={done} onChange={onToggle} />
      </span>
      <label role="cell" htmlFor={cbId} className="pv-imp-id mono">
        {item.identifier}
      </label>
      <label role="cell" htmlFor={cbId} className="pv-imp-title">
        <span className="ellipsis">{item.title}</span>
        <span className="pv-imp-meta">
          <ExtStateLabel state={item.state} />
          {item.labels.length > 0 && <span className="pv-imp-labels ellipsis">{item.labels.join(", ")}</span>}
        </span>
      </label>
      <span role="cell" className="pv-imp-repo">
        {done ? (
          <span className="pv-hint">Already imported</span>
        ) : repos.length === 0 ? (
          <span className="pv-hint">No repos</span>
        ) : (
          <>
            <select
              className={`input pv-select pv-select-sm${checked && !repoId ? " input-invalid" : ""}`}
              aria-label={`Repo for ${item.identifier}`}
              value={repoId}
              onChange={(e) => onRepo(e.target.value)}
            >
              {!repoId && <option value="">Pick repo</option>}
              {repos.map((r) => (
                <option key={r.id} value={r.id}>
                  {r.name}
                </option>
              ))}
            </select>
            {why && <span className="pv-kind">{why}</span>}
          </>
        )}
      </span>
    </div>
  );
}

/** Mismo criterio que `suggest_repo` en Rust: label igual sin distinguir mayúsculas. */
function ruleMatches(link: SourceLink, item: ImportableItem): boolean {
  return link.repoRules.some((r) =>
    item.labels.some((l) => l.trim().toLowerCase() === r.label.trim().toLowerCase()),
  );
}
