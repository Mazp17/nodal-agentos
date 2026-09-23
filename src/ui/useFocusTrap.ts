import { useEffect, useLayoutEffect, useRef } from "react";

const FOCUSABLE =
  'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

/** Diálogos montados, el último arriba: sólo ése atiende teclado. */
const stack: HTMLElement[] = [];

/**
 * Foco atrapado para diálogos: al montar enfoca `[data-autofocus]` (o el primer
 * elemento enfocable), Tab/Shift+Tab ciclan dentro, Escape llama a `onEscape`, y al
 * desmontar el foco vuelve a donde estaba. El root necesita `tabIndex={-1}`.
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
    // En `document` (no en el root): si el elemento enfocado se desmonta, el foco cae
    // en el body y el diálogo igual tiene que responder a Escape y Tab.
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
      // Solo se devuelve el foco si sigue dentro del diálogo (o se perdió en el body).
      if (previous?.isConnected && (root.contains(document.activeElement) || document.activeElement === document.body)) {
        previous.focus();
      }
    };
  }, []);

  return ref;
}
