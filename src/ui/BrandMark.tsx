/**
 * Nodal mark (`src-tauri/icons/source/nodal-mark.svg`): ring and accent dot,
 * without the macOS icon tile.
 */
export function BrandMark({ size = 22 }: { size?: number }) {
  return (
    <svg className="brand-mark" width={size} height={size} viewBox="0 0 40 40" aria-hidden focusable="false">
      <circle cx="13.35" cy="19.95" r="7.25" style={{ fill: "none", stroke: "var(--brand-ring)", strokeWidth: 3.8 }} />
      <circle cx="26.65" cy="19.95" r="9.15" style={{ fill: "var(--brand-dot)" }} />
    </svg>
  );
}
