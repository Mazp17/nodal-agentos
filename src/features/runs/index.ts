// Public runs API for the shell (D) and the board/tasks (E).

export { RunsView, type RunsViewProps } from "./RunsView";
export { RunDetailView, type RunDetailViewProps } from "./RunDetailView";
export { RunDiffDrawer, type RunDiffDrawerProps } from "./RunDiffDrawer";
// `AttachedSessionDrawer` is not re-exported: the shell lazy-loads it (xterm is heavy).
export { closeAttachedSession, useAttachTarget, type AttachTarget } from "./attachStore";
export { RunMonitor, type RunMonitorProps } from "./RunMonitor";
export { AgentTranscript } from "./AgentTranscript";
export { LaunchBlockerNotice, launchErrorHint, useLaunchBlocker } from "./LaunchBlockerNotice";
export { PhaseSegments, RunBadge, useRunView } from "./RunBadge";
export { useRunActions, type RunActions } from "./actions";
export {
  awaitingConfirmation,
  executorKindLabel,
  executorLabel,
  isActive,
  prNumber,
  runName,
  runStatusLabel,
  runTaskRef,
  type BadgeTone,
  type RunPhase,
  type RunsTab,
  type RunView,
} from "./status";
export {
  projectIdOf,
  refreshRuns,
  useQueueSummary,
  useRun,
  useRuns,
  type QueueSummary,
  type RunsFilter,
  type RunsState,
} from "../../domain/hooks/runs";
