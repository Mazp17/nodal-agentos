import { memo } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";
import { EXTERNAL, ExternalLink } from "./ExternalLink";
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

const components: Components = {
  a: ({ href, children }) => <ExternalLink url={href} className="md-link">{children}</ExternalLink>,
  img: ({ src, alt }) => {
    const label = alt ? `Image: ${alt}` : "Image";
    const url = typeof src === "string" ? src : undefined;
    return (
      <ExternalLink url={url} className="md-link md-img-link">
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
