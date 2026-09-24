// STUB(F2-D): replaced by E
import type { Task } from "../../domain/types";

export function TasksView(_props: {
  projectId: string;
  onOpenTask: (taskId: string) => void;
  onOpenRun: (runId: string) => void;
}): null {
  return null;
}

export function TaskPanel(_props: {
  taskId: string;
  onClose: () => void;
  onOpenRun: (runId: string) => void;
  onOpenTask: (taskId: string) => void;
}): null {
  return null;
}

export function NewTaskDialog(_props: {
  /** `null`: el diálogo pide el proyecto. */
  projectId: string | null;
  onClose: () => void;
  onCreated: (task: Task) => void;
}): null {
  return null;
}
