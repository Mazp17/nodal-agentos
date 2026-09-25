import { memo } from "react";
import Markdown, { type Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";
import { EXTERNAL, ExternalLink } from "./ExternalLink";
import "./markdown.css";

/*
 * Third-party Markdown (Linear) rendered safely:
 * - No rehype-raw and `skipHtml` on: raw HTML is dropped and never reaches the DOM.
 * - react-markdown's default `urlTransform`: neutralizes `javascript:`, `data:`, etc.
 * - Links carry no real `href`: that way neither WebKit's native context menu
 *   ("Open Link"), nor dragging, nor middle-click can navigate the webview. They open
 *   in the system browser via plugin-opener, and only if they're http(s) or mailto;
 *   the rest (relative, anchors, footnotes) shows as text.
 * - Images aren't loaded (the CSP would block them and they'd leak the IP to
 *   third-party hosts): they show as an external link.
 */

const components: Components = {
  a: ({ href, children, node }) => {
    // `[![alt](img)](url)`: the image already shows as a link; nesting <a> would open two URLs.
    const img = node?.children.find((c) => c.type === "element" && c.tagName === "img");
    if (img && img.type === "element") {
      const alt = typeof img.properties.alt === "string" ? img.properties.alt : "";
      return (
        <ExternalLink url={href} className="md-link">
          {alt ? `Image: ${alt}` : "Image"} ↗
        </ExternalLink>
      );
    }
    return (
      <ExternalLink url={href} className="md-link">
        {children}
      </ExternalLink>
    );
  },
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
// Agent output is often plain text with single line breaks; keep them.
const pluginsWithBreaks = [remarkGfm, remarkBreaks];

export const SafeMarkdown = memo(function SafeMarkdown({
  text,
  className,
  breaks = false,
}: {
  text: string;
  className?: string;
  /** Render single line breaks as <br>, for plain-text agent output. */
  breaks?: boolean;
}) {
  return (
    <div className={`md ${className ?? ""}`}>
      <Markdown remarkPlugins={breaks ? pluginsWithBreaks : plugins} skipHtml components={components}>
        {text}
      </Markdown>
    </div>
  );
});
