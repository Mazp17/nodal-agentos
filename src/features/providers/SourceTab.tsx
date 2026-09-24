import { useEffect, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { getTask, listTasks, syncNow, unlinkTask } from "../../domain/api";
import type { Task, TaskSource } from "../../domain/types";
import { errorText, useNow, useSourceLinks } from "../../domain/hooks/providers";
import { useToast } from "../../ui/Toasts";
import { IssueDetailCache, useIssueDetail } from "../linear/issueDetail";
import { Comments, DetailSections } from "../linear/IssueDetailSections";
import { InlineConfirm, ProviderMark } from "./parts";
import { ExtStateLabel } from "./StateMapEditor";
import { formatAgo, isUnmappedError, providerName, unmappedStateName } from "./meta";

export interface SourceTabProps {
  task: Task;
  /** La tarea cambió (sync o unlink): el padre reemplaza la suya. */
  onTaskChange?: (task: Task) => void;
  /** Abre otra tarea de Nodal (sub-issues y relaciones ya importadas). */
  onOpenTask?: (taskId: string) => void;
  /** "Review mapping": lleva a Project settings → Sources. */
  onReviewMapping?: () => void;
}

/** Pestaña Source del detalle de tarea: vínculo con el proveedor y detalle de la issue. */
export function SourceTab({ task, onTaskChange, onOpenTask, onReviewMapping }: SourceTabProps) {
  const src = task.source;
  if (!src) {
    return (
      <div className="pv-src">
        <div className="pv-empty">This task was created in Nodal. It isn't linked to a task manager.</div>
      </div>
    );
  }
  return (
    <LinkedSource
      key={src.externalId}
      task={task}
      src={src}
      onTaskChange={onTaskChange}
      onOpenTask={onOpenTask}
      onReviewMapping={onReviewMapping}
    />
  );
}

function LinkedSource({
  task,
  src,
  onTaskChange,
  onOpenTask,
  onReviewMapping,
}: SourceTabProps & { src: TaskSource }) {
  const toast = useToast();
  const now = useNow();
  const prov = providerName(src.provider);
  const links = useSourceLinks(task.projectId);
  const link = links.data?.find((l) => l.id === src.linkId) ?? null;
  const [syncing, setSyncing] = useState(false);
  const [confirmUnlink, setConfirmUnlink] = useState(false);
  const [unlinking, setUnlinking] = useState(false);

  const unmappedByError = isUnmappedError(src.syncError);
  // Estado registrado que dejó de estar en el mapeo confirmado (p. ej. se editó el mapeo).
  const unmappedByMap =
    !!link && link.stateMap.confirmedAt !== null && !!src.externalState && !(src.externalState.id in link.stateMap.pull);
  const unmapped = unmappedByError || unmappedByMap;
  const failed = !!src.syncError && !unmappedByError;

  const syncLabel = syncing
    ? "Syncing…"
    : failed
      ? "Sync failed"
      : src.lastSyncedAt
        ? `Synced ${formatAgo(src.lastSyncedAt, now)}`
        : "Not synced yet";
  const syncTone = syncing ? "accent" : failed ? "danger" : "muted";

  const retry = async () => {
    setSyncing(true);
    try {
      const r = await syncNow(src.linkId);
      const fresh = await getTask(task.id);
      onTaskChange?.(fresh);
      if (r.errors.length) toast("Sync finished with errors", r.errors.join("\n"), "danger");
    } catch (err) {
      toast("Sync failed", errorText(err), "danger");
    } finally {
      setSyncing(false);
    }
  };

  const unlink = async () => {
    setUnlinking(true);
    try {
      const t = await unlinkTask(task.id);
      toast(`${src.identifier} unlinked`, `${src.identifier} stays in ${prov}; this task is local now.`, "ok");
      onTaskChange?.(t);
    } catch (err) {
      toast("Could not unlink", errorText(err), "danger");
      setUnlinking(false);
    }
  };

  const open = (url: string) => openUrl(url).catch((err) => toast("Could not open the link", errorText(err), "danger"));

  return (
    <div className="pv-src">
      <div className="pv-src-head">
        <ProviderMark />
        <a
          href={src.url}
          className="mono pv-src-id"
          onClick={(e) => {
            e.preventDefault();
            void open(src.url);
          }}
          aria-label={`${src.identifier}, open in ${prov}`}
        >
          {src.identifier} ↗
        </a>
        {src.externalState && <ExtStateLabel state={src.externalState} />}
        <span className={`pv-sync tone-${syncTone}`} role="status">
          <span className={`dot dot-sm${syncing ? " pulse" : ""}`} aria-hidden />
          {syncLabel}
        </span>
      </div>

      {unmapped && (
        <div className="pv-alert pv-alert-warn" role="status">
          <span className="pv-alert-text">
            External state not mapped
            {unmappedByError && src.syncError && unmappedStateName(src.syncError)
              ? `: "${unmappedStateName(src.syncError)}"`
              : ""}
            . The task keeps its current status until the state is mapped.
          </span>
          {onReviewMapping && (
            <button type="button" className="btn btn-sm" onClick={onReviewMapping}>
              Review mapping
            </button>
          )}
        </div>
      )}
      {failed && (
        <div className="pv-alert pv-alert-danger" role="alert">
          <span className="pv-alert-text">{src.syncError}</span>
          <button type="button" className="btn btn-sm" disabled={syncing} onClick={() => void retry()}>
            Retry
          </button>
        </div>
      )}
      {!failed && !syncing && (
        <div className="pv-actions">
          <button type="button" className="btn btn-ghost btn-sm" onClick={() => void retry()}>
            Sync now
          </button>
        </div>
      )}

      {src.provider === "linear" ? (
        <LinearDetail
          issueId={src.externalId}
          url={src.url}
          projectId={task.projectId}
          onOpenTask={onOpenTask}
        />
      ) : (
        <div className="pv-empty">Details for {prov} issues open in {prov}.</div>
      )}

      <div className="pv-src-foot">
        {confirmUnlink ? (
          <InlineConfirm
            text={`Unlink ${src.identifier}? It stays in ${prov}; this task becomes local and stops syncing.`}
            confirmLabel="Unlink"
            busy={unlinking}
            onConfirm={() => void unlink()}
            onCancel={() => setConfirmUnlink(false)}
          />
        ) : (
          <button type="button" className="btn btn-sm" onClick={() => setConfirmUnlink(true)}>
            Unlink
          </button>
        )}
      </div>
    </div>
  );
}

function LinearDetail({
  issueId,
  url,
  projectId,
  onOpenTask,
}: {
  issueId: string;
  url: string;
  projectId: string;
  onOpenTask?: (taskId: string) => void;
}) {
  const [cache] = useState(() => new IssueDetailCache());
  const [state, retry] = useIssueDetail(issueId, cache);
  const byExternal = useTasksByExternalId(onOpenTask ? projectId : null);

  if (state.status === "loading") {
    return (
      <div className="ip-loading" aria-busy="true" aria-label="Loading issue details">
        <span className="ip-skel" style={{ width: "92%" }} />
        <span className="ip-skel" style={{ width: "78%" }} />
        <span className="ip-skel" style={{ width: "55%" }} />
      </div>
    );
  }
  if (state.status === "error") {
    return (
      <div className="pv-alert pv-alert-danger" role="alert">
        <span className="pv-alert-text">Could not load issue details. {state.error.message}</span>
        <button type="button" className="btn btn-sm" onClick={retry}>
          Retry
        </button>
      </div>
    );
  }
  return (
    <>
      <DetailSections
        detail={state.detail}
        isOnBoard={(id) => byExternal.has(id)}
        onOpenIssue={(id) => {
          const t = byExternal.get(id);
          if (t && onOpenTask) onOpenTask(t);
        }}
      />
      <Comments detail={state.detail} url={url} />
    </>
  );
}

/** externalId → id de tarea, para navegar a sub-issues y relaciones ya importadas. */
function useTasksByExternalId(projectId: string | null): Map<string, string> {
  const [map, setMap] = useState<Map<string, string>>(new Map());
  useEffect(() => {
    if (!projectId) return;
    let alive = true;
    listTasks(projectId).then(
      (tasks) => {
        if (!alive) return;
        const m = new Map<string, string>();
        for (const t of tasks) if (t.source) m.set(t.source.externalId, t.id);
        setMap(m);
      },
      (err) => console.error("listTasks", err),
    );
    return () => {
      alive = false;
    };
  }, [projectId]);
  return map;
}
