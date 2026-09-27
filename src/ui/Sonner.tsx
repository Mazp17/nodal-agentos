// shadcn/ui's Sonner Toaster with our own CSS instead of Tailwind. Nodal is dark-only, so the
// theme is fixed instead of read from next-themes.

import type { CSSProperties } from "react";
import { Toaster as Sonner, type ToasterProps } from "sonner";
import "./sonner.css";

/** The tone's dot (colored by `--tone` in sonner.css), used instead of icons. */
export const TONE_DOT = <span className="dot" aria-hidden />;

export function Toaster(props: ToasterProps) {
  return (
    <Sonner
      theme="dark"
      className="toaster"
      position="bottom-right"
      offset="var(--sp-6)"
      visibleToasts={4}
      closeButton
      icons={{ success: TONE_DOT, info: TONE_DOT, warning: TONE_DOT, error: TONE_DOT }}
      style={
        {
          "--normal-bg": "var(--surface-float)",
          "--normal-bg-hover": "var(--surface-float)",
          "--normal-text": "var(--text)",
          "--normal-border": "var(--border-3)",
          "--normal-border-hover": "var(--border-3)",
          "--border-radius": "var(--r-panel)",
          "--width": "310px",
        } as CSSProperties
      }
      {...props}
    />
  );
}
