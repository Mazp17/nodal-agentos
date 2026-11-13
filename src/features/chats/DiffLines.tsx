import type { DiffHunk, FileDiff } from "../../domain/api";
import "./diff.css";

/** `@@ -12,6 +14,8 @@ fn name` → where the hunk starts (new side) and the code context git names. */
function hunkLabel(h: DiffHunk): { line: string | null; context: string } {
  const m = /^@@ -(\d+)(?:,\d+)? \+(\d+)(?:,\d+)? @@ ?(.*)$/.exec(h.header);
  return m ? { line: m[2]!, context: m[3]!.trim() } : { line: null, context: h.header };
}

/**
 * One file's hunks. Full: old and new line numbers (file and commit views). `compact`: one
 * number column (the side the line lives on) and wrapped long lines, for the narrow chat.
 */
export function DiffLines({ file, className, compact = false }: { file: FileDiff; className?: string; compact?: boolean }) {
  return (
    <div className={`diff-lines ${compact ? "compact" : ""} ${className ?? ""}`} role="table" aria-label={`Diff of ${file.path}`}>
      {file.binary ? (
        <p className="diff-note">Binary file; no text diff.</p>
      ) : file.hunks.length === 0 ? (
        <p className="diff-note">No content changes.</p>
      ) : (
        file.hunks.map((h, hi) => {
          const { line, context } = hunkLabel(h);
          return (
            <div key={hi} role="rowgroup" className="diff-group">
              <div className="diff-hunk" role="row">
                {line && <span className="diff-hunk-at">Line {line}</span>}
                {context && <span className="diff-hunk-ctx">{context}</span>}
              </div>
              {h.lines.map((l, li) => (
                <div key={li} className={`diff-line dl-${l.kind}`} role="row">
                  {compact ? (
                    <span className="dl-no">{(l.kind === "del" ? l.oldNo : l.newNo) ?? ""}</span>
                  ) : (
                    <>
                      <span className="dl-no">{l.oldNo ?? ""}</span>
                      <span className="dl-no">{l.newNo ?? ""}</span>
                    </>
                  )}
                  <span className="dl-sign" aria-label={l.kind === "add" ? "added" : l.kind === "del" ? "removed" : undefined}>
                    {l.kind === "add" ? "+" : l.kind === "del" ? "−" : ""}
                  </span>
                  <span className="dl-text">{l.text || " "}</span>
                </div>
              ))}
            </div>
          );
        })
      )}
    </div>
  );
}
