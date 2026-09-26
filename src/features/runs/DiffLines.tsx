import type { FileDiff } from "../../domain/api";
import "./diff.css";

/** One file's hunks with old/new line numbers. Shared by the run diff and chat tool calls. */
export function DiffLines({ file, className }: { file: FileDiff; className?: string }) {
  return (
    <div className={`diff-lines ${className ?? ""}`} role="table" aria-label={`Diff of ${file.path}`}>
      {file.binary ? (
        <p className="diff-note">Binary file; no text diff.</p>
      ) : file.hunks.length === 0 ? (
        <p className="diff-note">No content changes.</p>
      ) : (
        file.hunks.map((h, hi) => (
          <div key={hi} role="rowgroup">
            <div className="diff-hunk" role="row">
              {h.header}
            </div>
            {h.lines.map((l, li) => (
              <div key={li} className={`diff-line dl-${l.kind}`} role="row">
                <span className="dl-no">{l.oldNo ?? ""}</span>
                <span className="dl-no">{l.newNo ?? ""}</span>
                <span className="dl-sign" aria-hidden>
                  {l.kind === "add" ? "+" : l.kind === "del" ? "−" : ""}
                </span>
                <span className="dl-text">{l.text || " "}</span>
              </div>
            ))}
          </div>
        ))
      )}
    </div>
  );
}
