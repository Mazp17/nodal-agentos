// TEMP(F2-E): shell mínimo para que la rama de E compile y se pueda probar el board. Lo
// reemplaza D (shell/AppShell.tsx + useNav.ts); de acá solo importa el cableado de abajo.
import { useState } from "react";
import { useProjects } from "./domain/hooks/tasks";
import { BoardView } from "./features/board";
import { NewTaskDialog, TaskPanel, TasksView } from "./features/tasks";
import { ToastProvider, useToast } from "./ui/Toasts";
import "./shell/shell.css";

type Page = "board" | "tasks";

function Shell() {
  const push = useToast();
  const projects = useProjects();
  const [projectId, setProjectId] = useState<string | null>(null);
  const [page, setPage] = useState<Page>("board");
  const [taskId, setTaskId] = useState<string | null>(null);
  const [newTask, setNewTask] = useState(false);

  // Sin RunDetail/RunDiff en esta rama (los trae G).
  const openRun = (runId: string) => push("Run", runId, "muted");

  return (
    <div style={{ display: "flex", height: "100vh" }}>
      <nav aria-label="Projects" style={{ width: 200, padding: 12, display: "flex", flexDirection: "column", gap: 4 }}>
        <button type="button" className="btn btn-ghost" onClick={() => setProjectId(null)}>
          All projects
        </button>
        {(projects.data ?? []).map((p) => (
          <button key={p.id} type="button" className="btn btn-ghost" onClick={() => setProjectId(p.id)}>
            {p.name}
          </button>
        ))}
      </nav>
      <main style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column" }}>
        <div style={{ display: "flex", gap: 8, padding: 8 }}>
          <button type="button" className="btn btn-sm" onClick={() => setPage("board")}>
            Board
          </button>
          <button type="button" className="btn btn-sm" onClick={() => setPage("tasks")}>
            Tasks
          </button>
          <span style={{ flex: 1 }} />
          <button type="button" className="btn btn-sm btn-primary" onClick={() => setNewTask(true)}>
            New task
          </button>
        </div>
        {page === "board" ? (
          <BoardView projectId={projectId} onOpenTask={setTaskId} onOpenRun={openRun} />
        ) : (
          <TasksView projectId={projectId} onOpenTask={setTaskId} />
        )}
      </main>
      {taskId && (
        <TaskPanel
          key={taskId}
          taskId={taskId}
          onClose={() => setTaskId(null)}
          onOpenRun={openRun}
          onOpenDiff={openRun}
          onOpenTask={setTaskId}
        />
      )}
      {newTask && <NewTaskDialog projectId={projectId} onClose={() => setNewTask(false)} onSaved={() => setNewTask(false)} />}
    </div>
  );
}

export default function App() {
  return (
    <ToastProvider>
      <Shell />
    </ToastProvider>
  );
}
