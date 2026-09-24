// Estado global de trabajo para el shell: píldora "N/M running · K need you · Q queued",
// contadores del sidebar y búsqueda de la paleta. Sondea mientras la ventana está visible.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { getSettings, listTaskRuns, listTasks } from "../domain/api";
import type { Run, Settings, Task } from "../domain/types";

const POLL_MS = 4000;
const ACTIVE: ReadonlySet<Run["status"]> = new Set(["launching", "launched"]);

export interface WorkStatus {
  loaded: boolean;
  error: string | null;
  tasks: Task[];
  settings: Settings | null;
  /** Runs `launching`/`launched` (trabajo y revisión). */
  active: Run[];
  /** En cola, en orden de salida (sin los migrados que esperan confirmación). */
  queued: Run[];
  /** Runs migrados en cola: no salen hasta confirmarlos. */
  awaitingConfirm: Run[];
  /** Tareas bloqueadas (revisión fallida, agente bloqueado, run detenido). */
  blocked: Task[];
  /** Blocked + migrados por confirmar. */
  needYou: number;
  /** Tareas abiertas (ni Done ni Canceled) por proyecto y total. */
  openByProject: Map<string, number>;
  openTotal: number;
  activeByProject: Map<string, number>;
  /** Tarea → run activo, para la paleta ("Run X" solo si no corre). */
  activeTaskIds: ReadonlySet<string>;
  refresh: () => Promise<void>;
  refreshSettings: () => Promise<void>;
}

export function useWorkStatus(): WorkStatus {
  const [runs, setRuns] = useState<Run[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const seq = useRef(0);

  const refresh = useCallback(async () => {
    const id = ++seq.current;
    try {
      const [rs, ts] = await Promise.all([listTaskRuns(null), listTasks(null)]);
      if (id !== seq.current) return;
      setRuns(rs);
      setTasks(ts);
      setError(null);
    } catch (e) {
      if (id === seq.current) setError(String(e));
    } finally {
      if (id === seq.current) setLoaded(true);
    }
  }, []);

  const refreshSettings = useCallback(async () => {
    try {
      setSettings(await getSettings());
    } catch {
      /* se reintenta en el próximo refresh de Settings */
    }
  }, []);

  useEffect(() => {
    void refresh();
    void refreshSettings();
    let timer: ReturnType<typeof setInterval> | null = null;
    const start = () => {
      if (timer === null) timer = setInterval(() => void refresh(), POLL_MS);
    };
    const stop = () => {
      if (timer !== null) clearInterval(timer);
      timer = null;
    };
    const onVis = () => {
      if (document.hidden) stop();
      else {
        void refresh();
        start();
      }
    };
    if (!document.hidden) start();
    document.addEventListener("visibilitychange", onVis);
    return () => {
      stop();
      document.removeEventListener("visibilitychange", onVis);
    };
  }, [refresh, refreshSettings]);

  return useMemo((): WorkStatus => {
    const taskById = new Map(tasks.map((t) => [t.id, t]));
    const active = runs.filter((r) => ACTIVE.has(r.status));
    const queuedAll = runs.filter((r) => r.status === "queued").sort((a, b) => a.queuePosition - b.queuePosition);
    const queued = queuedAll.filter((r) => !r.legacyLabel);
    const awaitingConfirm = queuedAll.filter((r) => r.legacyLabel);
    const blocked = tasks.filter((t) => t.status === "blocked");
    const openByProject = new Map<string, number>();
    let openTotal = 0;
    for (const t of tasks) {
      if (t.status === "done" || t.status === "canceled") continue;
      openTotal++;
      openByProject.set(t.projectId, (openByProject.get(t.projectId) ?? 0) + 1);
    }
    const activeByProject = new Map<string, number>();
    const activeTaskIds = new Set<string>();
    for (const r of [...active, ...queuedAll]) {
      if (r.taskId) activeTaskIds.add(r.taskId);
    }
    for (const r of active) {
      const p = r.taskId ? taskById.get(r.taskId)?.projectId : undefined;
      if (p) activeByProject.set(p, (activeByProject.get(p) ?? 0) + 1);
    }
    return {
      loaded,
      error,
      tasks,
      settings,
      active,
      queued,
      awaitingConfirm,
      blocked,
      needYou: blocked.length + awaitingConfirm.length,
      openByProject,
      openTotal,
      activeByProject,
      activeTaskIds,
      refresh,
      refreshSettings,
    };
  }, [runs, tasks, settings, loaded, error, refresh, refreshSettings]);
}
