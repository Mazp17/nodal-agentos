import { createContext, useCallback, useContext, useRef, useState, type ReactNode } from "react";

/** Mismos tonos que los badges (`.tone-*`), más `info`. */
export type ToastTone = "accent" | "ok" | "warn" | "danger" | "muted" | "info";

interface Toast {
  id: number;
  title: string;
  body?: string;
  tone: ToastTone;
}

type Push = (title: string, body?: string, tone?: ToastTone) => void;

const ToastContext = createContext<Push>(() => {});

/** `toast(title, body?, tone?)`: aviso global abajo a la derecha. */
export function useToast(): Push {
  return useContext(ToastContext);
}

const MAX_TOASTS = 4;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toasts, setToasts] = useState<Toast[]>([]);
  const seq = useRef(0);

  const dismiss = useCallback((id: number) => setToasts((t) => t.filter((x) => x.id !== id)), []);

  const push = useCallback<Push>(
    (title, body, tone = "info") => {
      const id = ++seq.current;
      setToasts((t) => [...t.slice(-(MAX_TOASTS - 1)), { id, title, body, tone }]);
      // Los errores quedan hasta cerrarlos: suelen traer texto para leer (WCAG 2.2.1).
      if (tone !== "danger") setTimeout(() => dismiss(id), 5000);
    },
    [dismiss],
  );

  return (
    <ToastContext.Provider value={push}>
      {children}
      <div className="toasts" aria-live="polite">
        {toasts.map((t) => (
          <div key={t.id} className={`toast tone-${t.tone}`}>
            <span className="dot" aria-hidden />
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
