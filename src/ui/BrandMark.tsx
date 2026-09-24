/**
 * Marca de Nodal: el logo "Orbit" (`src-tauri/icons/source/icon.svg`) sin el cuerpo del
 * ícono de macOS — núcleo con tres agentes en órbita —, en la paleta cálida.
 */
export function BrandMark({ size = 22 }: { size?: number }) {
  return (
    <svg className="brand-mark" width={size} height={size} viewBox="212 212 600 600" aria-hidden focusable="false">
      <circle cx="512" cy="512" r="242.6" style={{ fill: "none", stroke: "var(--brand-ring)", strokeWidth: 22 }} />
      <circle cx="512" cy="512" r="91.6" style={{ fill: "var(--brand-core)" }} />
      <circle cx="512" cy="264.8" r="45.8" style={{ fill: "var(--brand-sat)" }} />
      <circle cx="713.4" cy="621.9" r="45.8" style={{ fill: "var(--brand-sat)" }} />
      <circle cx="310.6" cy="621.9" r="45.8" style={{ fill: "var(--brand-sat)" }} />
    </svg>
  );
}
