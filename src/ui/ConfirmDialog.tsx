import { createContext, useCallback, useContext, useEffect, useId, useRef, useState, type ReactNode } from "react";
import { useFocusTrap } from "./useFocusTrap";
import "./confirm.css";

/*
 * Our own modal confirmation. Don't use `window.confirm`/`alert`/`prompt`: in Tauri,
 * tauri-plugin-dialog replaces them with async versions (they return a Promise, always
 * truthy) and the command isn't allowed in the capabilities anyway. See the
 * `lint:no-native-dialogs` script.
 */

export interface ConfirmOptions {
  title: string;
  /** What is lost or what happens: name it concretely. */
  body?: ReactNode;
  confirmLabel: string;
  cancelLabel?: string;
  /** Red confirm button (default true). */
  destructive?: boolean;
}

type Confirm = (opts: ConfirmOptions) => Promise<boolean>;

const ConfirmContext = createContext<Confirm>(() => {
  console.error("useConfirm() outside <ConfirmProvider>: the action was cancelled.");
  return Promise.resolve(false);
});

/** `const ask = useConfirm(); await ask({ title, body, confirmLabel })` → true if the user confirmed. */
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
        // One at a time: a new one cancels the previous.
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

  // If the provider unmounts with one open, don't leave the Promise hanging.
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
  // The shell's global shortcuts (⌘K, ⌘1…9, ⌘,) ignore events with defaultPrevented:
  // while confirming, the palette doesn't open and nothing navigates underneath.
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
          {/* Initial focus on Cancel: a reflexive Enter destroys nothing. */}
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
