import { toast } from "sonner";
import { TONE_DOT } from "./Sonner";

/** Same tones as the badges (`.tone-*`), plus `info`. */
export type ToastTone = "accent" | "ok" | "warn" | "danger" | "muted" | "info";

type Push = (title: string, body?: string, tone?: ToastTone) => void;

const push: Push = (title, body, tone = "info") => {
  // Errors stay until dismissed: they usually carry text to read (WCAG 2.2.1).
  const opts = { description: body, duration: tone === "danger" ? Infinity : 5000 };
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

/** `toast(title, body?, tone?)`: global notice at the bottom right (needs `<Toaster />` mounted). */
export function useToast(): Push {
  return push;
}
