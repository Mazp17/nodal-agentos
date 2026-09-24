import type { KeyboardEvent, ReactNode } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";

export const EXTERNAL = /^(https?:|mailto:)/i;

/**
 * Link externo sin `href` real: así ni el menú contextual nativo de WebKit ("Open Link"),
 * ni arrastrar, ni el clic medio pueden navegar el webview. Se abre en el navegador del
 * sistema con plugin-opener, y sólo si es http(s) o mailto; si no, se muestra como texto.
 * Usar esto en vez de `<a href="http…">` en toda la app.
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
  /** aria-label, si el texto visible no alcanza. */
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
