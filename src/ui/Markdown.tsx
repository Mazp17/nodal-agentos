import { memo } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import "./markdown.css";

/*
 * Markdown de terceros (Linear) renderizado de forma segura:
 * - Sin rehype-raw y con `skipHtml`: el HTML crudo se descarta, nunca llega al DOM.
 * - `urlTransform` por defecto de react-markdown: neutraliza `javascript:`, `data:`, etc.
 * - Los links nunca navegan el webview: se abren en el navegador del sistema con
 *   plugin-opener, y sólo si son http(s) o mailto.
 * - Las imágenes no se cargan (la CSP las bloquearía y además filtrarían la IP a
 *   hosts de terceros): se muestran como link externo.
 */

const EXTERNAL = /^(https?:|mailto:)/i;

function openExternal(href: string | undefined) {
  if (href && EXTERNAL.test(href)) {
    openUrl(href).catch((err) => console.error("openUrl", err));
  }
}

const components: Components = {
  a: ({ href, children }) => (
    <a
      href={href}
      title={href}
      onClick={(e) => {
        e.preventDefault();
        openExternal(href);
      }}
      onAuxClick={(e) => e.preventDefault()}
    >
      {children}
    </a>
  ),
  img: ({ src, alt }) => {
    const url = typeof src === "string" ? src : undefined;
    const label = alt ? `Image: ${alt}` : "Image";
    return url && EXTERNAL.test(url) ? (
      <a
        href={url}
        title={url}
        className="md-img-link"
        onClick={(e) => {
          e.preventDefault();
          openExternal(url);
        }}
        onAuxClick={(e) => e.preventDefault()}
      >
        {label} ↗
      </a>
    ) : (
      <span className="md-img-link">{label}</span>
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
