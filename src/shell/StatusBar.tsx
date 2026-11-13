import { useEffect } from "react";
import { getPlanUsage, type PlanUsage, type PlanWindow } from "../domain/api";
import { POLL, usePolled } from "../domain/hooks/store";
import { formatDateTime } from "../lib/format";
import { Tooltip, TooltipContent, TooltipTrigger } from "../ui/Tooltip";

/** Claude plan usage (5-hour and weekly windows) at the bottom of every view. */
export function StatusBar() {
  const usage = usePolled<PlanUsage>("plan-usage", getPlanUsage, [], POLL.slow);
  const { refresh } = usage;
  useEffect(() => {
    // The store only refreshes on visibilitychange; a visible window regaining focus doesn't fire it.
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  const u = usage.data;
  if (!u) return <footer className="status-bar" aria-label="Claude plan usage" />;
  return (
    <footer className="status-bar" aria-label="Claude plan usage">
      {u.status === "available" ? (
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="status-meters" tabIndex={0}>
              <Meter label="5h" w={u.fiveHour} reset={(d) => d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })} />
              <Meter label="Week" w={u.weekly} reset={(d) => d.toLocaleDateString([], { weekday: "short" })} />
            </span>
          </TooltipTrigger>
          <TooltipContent side="top">Claude plan usage as of {formatDateTime(u.updatedAt)} (last reply of a Nodal chat or task)</TooltipContent>
        </Tooltip>
      ) : (
        <Tooltip>
          <TooltipTrigger asChild>
            <span className="status-bar-muted" tabIndex={0}>Usage unavailable</span>
          </TooltipTrigger>
          <TooltipContent side="top">{u.reason}</TooltipContent>
        </Tooltip>
      )}
    </footer>
  );
}

function Meter({ label, w, reset }: { label: string; w: PlanWindow; reset: (d: Date) => string }) {
  const pct = Math.round(w.pct);
  const tone = pct >= 95 ? "danger" : pct >= 80 ? "warn" : "ok";
  const resets = w.resetsAt ? new Date(w.resetsAt) : null;
  return (
    <span className={`status-meter tone-${tone}`}>
      <span>
        {label} {pct}%{resets && <span className="status-bar-muted"> · resets {reset(resets)}</span>}
      </span>
      <span className="status-meter-track" role="progressbar" aria-label={`${label} usage`} aria-valuenow={pct} aria-valuemin={0} aria-valuemax={100}>
        <span className="status-meter-fill" style={{ width: `${Math.min(pct, 100)}%` }} />
      </span>
    </span>
  );
}
