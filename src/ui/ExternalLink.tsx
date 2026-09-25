import type { KeyboardEvent, ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

export const EXTERNAL = /^(https?:|mailto:)/i;

/**
 * External link without a real `href`: that way neither WebKit's native context menu
 * ("Open Link"), nor dragging, nor middle-click can navigate the webview. It opens in the
 * system browser via plugin-opener, and only if it's http(s) or mailto; otherwise it shows as text.
 * Use this instead of `<a href="http…">` throughout the app.
 */
export function ExternalLink({
  url,
  className,
  label,
  onError,
  children,
}: {
  url: string | undefined | null;
  className?: string;
  /** aria-label, if the visible text isn't enough. */
  label?: string;
  /** Default: console.error. */
  onError?: (err: unknown) => void;
  children: ReactNode;
}) {
  if (!url || !EXTERNAL.test(url)) return <span className={className}>{children}</span>;
  const open = () => {
    openUrl(url).catch(onError ?? ((err: unknown) => console.error("openUrl", err)));
  };
  return (
    <a
      role="link"
      tabIndex={0}
      title={url}
      aria-label={label}
      className={`ext-link ${className ?? ""}`}
      draggable={false}
      onClick={open}
      onKeyDown={(e: KeyboardEvent) => {
        if (e.key === "Enter") {
          e.preventDefault();
          open();
        }
      }}
    >
      {children}
    </a>
  );
}
