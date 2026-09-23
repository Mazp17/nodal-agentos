import { memo, type KeyboardEvent, type ReactNode } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./markdown.css";

/*
 * Markdown de terceros (Linear) renderizado de forma segura:
 * - Sin rehype-raw y con `skipHtml`: el HTML crudo se descarta, nunca llega al DOM.
 * - `urlTransform` por defecto de react-markdown: neutraliza `javascript:`, `data:`, etc.
 * - Los links no llevan `href` real: así ni el menú contextual nativo de WebKit
 *   ("Open Link"), ni arrastrar, ni el click medio pueden navegar el webview. Se abren
 *   en el navegador del sistema con plugin-opener, y sólo si son http(s) o mailto;
 *   el resto (relativos, anclas, footnotes) se muestra como texto.
 * - Las imágenes no se cargan (la CSP las bloquearía y además filtrarían la IP a
 *   hosts de terceros): se muestran como link externo.
 */

const EXTERNAL = /^(https?:|mailto:)/i;

function ExternalLink({ url, className, children }: { url: string | undefined; className?: string; children: ReactNode }) {
  if (!url || !EXTERNAL.test(url)) return <span className={className}>{children}</span>;
  const open = () => {
    openUrl(url).catch((err) => console.error("openUrl", err));
  };
  return (
    <a
      role="link"
      tabIndex={0}
      title={url}
      className={`md-link ${className ?? ""}`}
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

const components: Components = {
  a: ({ href, children }) => <ExternalLink url={href}>{children}</ExternalLink>,
  img: ({ src, alt }) => {
    const label = alt ? `Image: ${alt}` : "Image";
    const url = typeof src === "string" ? src : undefined;
    return (
      <ExternalLink url={url} className="md-img-link">
        {url && EXTERNAL.test(url) ? `${label} ↗` : label}
      </ExternalLink>
    );
  },
};

const plugins = [remarkGfm];

export const SafeMarkdown = memo(function SafeMarkdown({ text, className }: { text: string; className?: string }) {
  return (
    <div className={`md ${className ?? ""}`}>
      <Markdown remarkPlugins={plugins} skipHtml components={components}>
        {text}
      </Markdown>
    </div>
  );
});
