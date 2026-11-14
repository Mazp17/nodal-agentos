import { toast } from "sonner";
import { TONE_DOT } from "./Sonner";

/** Same tones as the badges (`.tone-*`), plus `info`. */
export type ToastTone = "accent" | "ok" | "warn" | "danger" | "muted" | "info";

/** A button on the toast (e.g. "Open"); clicking it also dismisses the toast. */
export interface ToastAction {
  label: string;
  onClick: () => void;
}

type Push = (title: string, body?: string, tone?: ToastTone, action?: ToastAction) => void;

const push: Push = (title, body, tone = "info", action) => {
  // Errors stay until dismissed: they usually carry text to read (WCAG 2.2.1).
  const opts = { description: body, duration: tone === "danger" ? Infinity : 5000, action };
  switch (tone) {
    case "ok":
      toast.success(title, opts);
      break;
    case "warn":
      toast.warning(title, opts);
      break;
    case "danger":
      toast.error(title, opts);
      break;
    case "info":
      toast.info(title, opts);
      break;
    default:
      // Tones without a Sonner type: the dot takes its color from the tone class.
      toast(title, { ...opts, icon: TONE_DOT, className: `tone-${tone}` });
  }
};

/** `toast(title, body?, tone?, action?)`: global notice at the bottom right (needs `<Toaster />` mounted). */
export function useToast(): Push {
  return push;
}
