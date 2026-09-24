import { createContext, useCallback, useContext, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useFocusTrap } from "./useFocusTrap";
import "./confirm.css";

/*
 * Confirmación modal propia. No usar `window.confirm`/`alert`/`prompt`: en Tauri,
 * tauri-plugin-dialog los reemplaza por versiones async (devuelven una Promise, siempre
 * truthy) y además el comando no está permitido en las capabilities. Ver el script
 * `lint:no-native-dialogs`.
 */

export interface ConfirmOptions {
  title: string;
  /** Qué se pierde o qué pasa: nombrarlo concretamente. */
  body?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  /** Botón de confirmar en rojo (default true). */
  destructive?: boolean;
}

type Confirm = (opts: ConfirmOptions) => Promise<boolean>;

const ConfirmContext = createContext<Confirm>(() => {
  console.error("useConfirm() outside <ConfirmProvider>: the action was cancelled.");
  return Promise.resolve(false);
});

/** `const ask = useConfirm(); await ask({ title, body, confirmLabel })` → true si el usuario confirmó. */
export function useConfirm(): Confirm {
  return useContext(ConfirmContext);
}

interface Pending extends ConfirmOptions {
  id: number;
  resolve: (ok: boolean) => void;
}

export function ConfirmProvider({ children }: { children: ReactNode }) {
  const [pending, setPending] = useState<Pending | null>(null);
  const current = useRef<Pending | null>(null);
  const seq = useRef(0);

  const confirm = useCallback<Confirm>(
    (opts) =>
      new Promise<boolean>((resolve) => {
        // Uno a la vez: uno nuevo cancela el anterior.
        current.current?.resolve(false);
        const p = { ...opts, id: ++seq.current, resolve };
        current.current = p;
        setPending(p);
      }),
    [],
  );

  const settle = useCallback((ok: boolean) => {
    const p = current.current;
    current.current = null;
    setPending(null);
    p?.resolve(ok);
  }, []);

  // Si el provider se desmonta con uno abierto, no dejar la Promise colgada.
  useEffect(() => () => current.current?.resolve(false), []);

  return (
    <ConfirmContext.Provider value={confirm}>
      {children}
      {pending && <ConfirmDialog key={pending.id} {...pending} onSettle={settle} />}
    </ConfirmContext.Provider>
  );
}

function ConfirmDialog({
  title,
  body,
  confirmLabel,
  cancelLabel = "Cancel",
  destructive = true,
  onSettle,
}: ConfirmOptions & { onSettle: (ok: boolean) => void }) {
  const ref = useFocusTrap<HTMLDivElement>(() => onSettle(false));
  const titleId = useId();
  // Los atajos globales del shell (⌘K, ⌘1…9, ⌘,) ignoran eventos con defaultPrevented:
  // mientras se confirma no se abre la paleta ni se navega por debajo.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && !/^[acvxz]$/i.test(e.key)) e.preventDefault();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, []);
  const bodyId = useId();
  return (
    <>
      <div className="confirm-scrim" onClick={() => onSettle(false)} aria-hidden />
      <div
        ref={ref}
        className="confirm"
        role="alertdialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-describedby={body ? bodyId : undefined}
        tabIndex={-1}
      >
        <h2 id={titleId} className="confirm-title">
          {title}
        </h2>
        {body && (
          <div id={bodyId} className="confirm-body">
            {body}
          </div>
        )}
        <div className="confirm-foot">
          {/* Foco inicial en Cancelar: Enter por reflejo no destruye nada. */}
          <button type="button" className="btn btn-ghost" data-autofocus onClick={() => onSettle(false)}>
            {cancelLabel}
          </button>
          <button type="button" className={destructive ? "btn btn-danger" : "btn btn-primary"} onClick={() => onSettle(true)}>
            {confirmLabel}
          </button>
        </div>
      </div>
    </>
  );
}
