import { useEffect, useLayoutEffect, useRef } from "react";

export const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Mounted dialogs, the latest on top: only that one handles the keyboard. */
const stack: HTMLElement[] = [];

/**
 * Focus trap for dialogs: on mount it focuses `[data-autofocus]` (or the first
 * focusable element), Tab/Shift+Tab cycle inside, Escape calls `onEscape`, and on
 * unmount focus returns to where it was. The root needs `tabIndex={-1}`.
 */
export function useFocusTrap<T extends HTMLElement>(onEscape?: () => void) {
  const ref = useRef<T>(null);
  const escRef = useRef(onEscape);
  useLayoutEffect(() => {
    escRef.current = onEscape;
  });

  useEffect(() => {
    const root = ref.current;
    if (!root) return;
    const previous = document.activeElement as HTMLElement | null;
    const focusables = () =>
      [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => el.getClientRects().length > 0);

    if (!root.contains(document.activeElement)) {
      (root.querySelector<HTMLElement>("[data-autofocus]") ?? focusables()[0] ?? root).focus();
    }

    stack.push(root);
    // On `document` (not the root): if the focused element unmounts, focus falls
    // to the body and the dialog still has to respond to Escape and Tab.
    const onKey = (e: KeyboardEvent) => {
      if (stack[stack.length - 1] !== root) return;
      if (e.key === "Escape" && escRef.current) {
        e.preventDefault();
        e.stopPropagation();
        escRef.current();
        return;
      }
      if (e.key !== "Tab") return;
      const list = focusables();
      if (list.length === 0) {
        e.preventDefault();
        return;
      }
      const first = list[0];
      const last = list[list.length - 1];
      const current = document.activeElement;
      if (!root.contains(current)) {
        e.preventDefault();
        (e.shiftKey ? last : first).focus();
      } else if (e.shiftKey && (current === first || current === root)) {
        e.preventDefault();
        last.focus();
      } else if (!e.shiftKey && current === last) {
        e.preventDefault();
        first.focus();
      }
    };
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("keydown", onKey);
      stack.splice(stack.indexOf(root), 1);
      // Focus is only returned if it's still inside the dialog (or was lost to the body).
      if (previous?.isConnected && (root.contains(document.activeElement) || document.activeElement === document.body)) {
        previous.focus();
      }
    };
  }, []);

  return ref;
}
