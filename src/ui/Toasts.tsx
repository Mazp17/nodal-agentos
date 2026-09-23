import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";
import type { BadgeTone } from "../features/runs/status";

interface Toast {
  id: number;
  title: string;
  body?: string;
  tone: BadgeTone;
}

type Push = (title: string, body?: string, tone?: BadgeTone) => void;

const ToastContext = createContext<Push>(() => {});

export function useToast(): Push {
  return useContext(ToastContext);
}

const MAX_TOASTS = 4;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const seq = useRef(0);

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const push = useCallback<Push>(
    (title, body, tone = "accent") => {
      const id = ++seq.current;
      setToasts((t) => [...t.slice(-(MAX_TOASTS - 1)), { id, title, body, tone }]);
      // Los errores quedan más tiempo: suelen traer texto para leer.
      setTimeout(() => dismiss(id), tone === "danger" ? 9000 : 5000);
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast tone-${t.tone}`} role={t.tone === "danger" ? "alert" : "status"}>
            <span className="dot dot-lg" aria-hidden />
            <div className="toast-body">
              <span className="toast-title">{t.title}</span>
              {t.body && <span className="toast-text">{t.body}</span>}
            </div>
            <button type="button" className="icon-btn" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              ✕
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}
