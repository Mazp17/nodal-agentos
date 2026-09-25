import { useEffect, useState } from "react";
import { saveStateMap, type MapOrigin, type SourceStatesReport } from "../../domain/api";
import { TASK_STATUSES, type ExternalState, type SourceLink, type StateMap, type TaskStatus } from "../../domain/types";
import { errorText, invalidateProviders } from "../../domain/hooks/providers";
import { useToast } from "../../ui/Toasts";
import { EXT_KIND_LABEL, ORIGIN_LABEL, STATUS_COLOR, STATUS_LABEL, plural, providerName } from "./meta";

type Pull = Record<string, TaskStatus>;
type Push = Partial<Record<TaskStatus, string | null>>;

/**
 * Editor for the state mapping in both directions. Starts from the proposal from
 * `source_states` (what's saved + suggestions for what's missing) and saves with
 * `save_state_map`, which confirms the mapping.
 */
export function StateMapEditor({
  link,
  report,
  onSaved,
  onCancel,
}: {
  link: SourceLink;
  report: SourceStatesReport;
  onSaved: (link: SourceLink) => void;
  onCancel: () => void;
}) {
  const toast = useToast();
  const prov = providerName(link.provider);
  const [pull, setPull] = useState<Pull>(report.proposal.pull);
  const [push, setPush] = useState<Push>(report.proposal.push);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // A new report (reload) replaces the draft.
  useEffect(() => {
    setPull(report.proposal.pull);
    setPush(report.proposal.push);
  }, [report]);

  const addedIds = new Set(report.added.map((s) => s.id));
  const pullEdited = (id: string) => pull[id] !== report.proposal.pull[id];
  const pushEdited = (st: TaskStatus) => (push[st] ?? null) !== (report.proposal.push[st] ?? null);
  const pushStatuses = TASK_STATUSES.filter((s) => s in report.proposal.push);
  const edited = report.states.some((s) => pullEdited(s.id)) || pushStatuses.some(pushEdited);
  const pending =
    Object.values(report.pullOrigin).some((o) => o !== "confirmed") ||
    Object.values(report.pushOrigin).some((o) => o !== "confirmed");
  const unmapped =
    Object.values(report.pullOrigin).filter((o) => o === "unmapped").length +
    Object.values(report.pushOrigin).filter((o) => o === "unmapped").length;

  const save = async (map: { pull: Pull; push: Push }) => {
    setSaving(true);
    setError(null);
    try {
      const full: StateMap = { ...report.proposal, pull: map.pull, push: map.push };
      const saved = await saveStateMap(link.id, full);
      invalidateProviders("links");
      toast("Mapping saved", `${prov} · ${link.scope.name}. Nodal now syncs state both ways.`, "ok");
      onSaved(saved);
    } catch (err) {
      setError(errorText(err));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="pv-map" aria-label="Status mapping">
      {report.added.length > 0 && (
        <div className="pv-alert pv-alert-warn" role="status">
          {plural(report.added.length, "new state")} in {prov}: {report.added.map((s) => s.name).join(", ")}. Pick a
          Nodal status for {report.added.length === 1 ? "it" : "them"} below. Until you save, tasks in{" "}
          {report.added.length === 1 ? "that state" : "those states"} keep their current status.
        </div>
      )}
      {report.removed.length > 0 && (
        <div className="pv-alert pv-alert-warn" role="status">
          Removed in {prov}: {report.removed.map((s) => s.name).join(", ")}. Nodal no longer pushes to{" "}
          {report.removed.length === 1 ? "it" : "them"}; pick another state if a row below needs one.
        </div>
      )}
      {link.stateMap.confirmedAt === null && (
        <p className="pv-hint">
          Nodal proposed this mapping from the states in {prov}. Until you confirm it, you can import, but Nodal
          won't push status changes.
        </p>
      )}

      <div className="pv-map-table" role="table" aria-label={`Pull: ${prov} to Nodal`}>
        <div className="pv-map-caption" role="caption">
          <span className="pv-map-dir">Pull</span>
          <span className="pv-hint">When an issue changes state in {prov}, the task moves to…</span>
        </div>
        <div className="pv-map-head" role="row">
          <span role="columnheader">{prov} state</span>
          <span aria-hidden />
          <span role="columnheader">Nodal status</span>
          <span role="columnheader" className="sr-only">
            Origin
          </span>
        </div>
        {report.states.map((s) => (
          <div key={s.id} role="row" className={`pv-map-row${addedIds.has(s.id) ? " is-new" : ""}`}>
            <span role="cell" className="pv-map-ext">
              <ExtStateLabel state={s} />
              <span className="pv-kind">{EXT_KIND_LABEL[s.kind]}</span>
              {addedIds.has(s.id) && <span className="pv-tag pv-tag-warn">New</span>}
            </span>
            <span aria-hidden className="pv-map-arrow">
              →
            </span>
            <span role="cell">
              <StatusSelect
                value={pull[s.id] ?? null}
                label={`Nodal status for ${s.name}`}
                onChange={(v) => setPull((p) => ({ ...p, [s.id]: v }))}
              />
            </span>
            <span role="cell">
              <OriginTag origin={report.pullOrigin[s.id]} edited={pullEdited(s.id)} />
            </span>
          </div>
        ))}
        {report.states.length === 0 && <div className="pv-empty">{prov} returned no states for this scope.</div>}
      </div>

      <div className="pv-map-table" role="table" aria-label={`Push: Nodal to ${prov}`}>
        <div className="pv-map-caption" role="caption">
          <span className="pv-map-dir">Push</span>
          <span className="pv-hint">When a run moves the task, Nodal sets the issue to… ("Don't sync" only comments)</span>
        </div>
        <div className="pv-map-head" role="row">
          <span role="columnheader">Nodal status</span>
          <span aria-hidden />
          <span role="columnheader">{prov} state</span>
          <span role="columnheader" className="sr-only">
            Origin
          </span>
        </div>
        {pushStatuses.map((st) => (
          <div key={st} role="row" className="pv-map-row">
            <span role="cell" className="pv-map-ext">
              <StatusLabel status={st} />
            </span>
            <span aria-hidden className="pv-map-arrow">
              →
            </span>
            <span role="cell">
              <select
                className="input pv-select pv-select-sm"
                aria-label={`${prov} state for ${STATUS_LABEL[st]}`}
                value={push[st] ?? ""}
                onChange={(e) => setPush((p) => ({ ...p, [st]: e.target.value || null }))}
              >
                <option value="">Don't sync</option>
                {report.states.map((s) => (
                  <option key={s.id} value={s.id}>
                    {s.name}
                  </option>
                ))}
              </select>
            </span>
            <span role="cell">
              <OriginTag origin={report.pushOrigin[st]} edited={pushEdited(st)} />
            </span>
          </div>
        ))}
      </div>

      {error && (
        <div className="pv-alert pv-alert-danger" role="alert">
          {error}
        </div>
      )}
      <div className="pv-actions pv-actions-end">
        {unmapped > 0 && <span className="pv-hint pv-grow">{plural(unmapped, "row")} unmapped</span>}
        <button type="button" className="btn btn-ghost btn-sm" onClick={onCancel} disabled={saving}>
          {edited ? "Discard" : "Close"}
        </button>
        {pending && !edited && (
          <button
            type="button"
            className="btn btn-primary btn-sm"
            disabled={saving}
            onClick={() => void save(report.proposal)}
          >
            {saving ? "Saving…" : "Accept suggestions"}
          </button>
        )}
        {(edited || !pending) && (
          <button type="button" className="btn btn-primary btn-sm" disabled={saving} onClick={() => void save({ pull, push })}>
            {saving ? "Saving…" : "Save mapping"}
          </button>
        )}
      </div>
    </div>
  );
}

export function ExtStateLabel({ state }: { state: ExternalState }) {
  return (
    <span className="pv-state">
      <span className="dot-ring pv-ring" style={{ color: state.color ?? "var(--text-dim)" }} aria-hidden />
      <span className="ellipsis">{state.name}</span>
    </span>
  );
}

export function StatusLabel({ status }: { status: TaskStatus }) {
  return (
    <span className="pv-state">
      <span
        className={`dot-ring pv-ring${status === "in_progress" ? " pv-ring-dashed" : ""}`}
        style={{ color: STATUS_COLOR[status] }}
        aria-hidden
      />
      {STATUS_LABEL[status]}
    </span>
  );
}

function StatusSelect({
  value,
  onChange,
  label,
}: {
  value: TaskStatus | null;
  onChange: (v: TaskStatus) => void;
  label: string;
}) {
  return (
    <select
      className="input pv-select pv-select-sm"
      aria-label={label}
      value={value ?? ""}
      onChange={(e) => e.target.value && onChange(e.target.value as TaskStatus)}
    >
      {value === null && (
        <option value="" disabled>
          Pick a status
        </option>
      )}
      {TASK_STATUSES.map((s) => (
        <option key={s} value={s}>
          {STATUS_LABEL[s]}
        </option>
      ))}
    </select>
  );
}

function OriginTag({ origin, edited }: { origin: MapOrigin | undefined; edited: boolean }) {
  if (edited) return <span className="pv-tag pv-tag-accent">Edited</span>;
  if (!origin) return null;
  const tone = origin === "confirmed" ? "pv-tag-ok" : origin === "unmapped" ? "pv-tag-warn" : "pv-tag-muted";
  return <span className={`pv-tag ${tone}`}>{ORIGIN_LABEL[origin]}</span>;
}
