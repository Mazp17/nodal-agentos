/** Logo de Agent Desk: núcleo con tres agentes en órbita. Escala con `size`. */
export function BrandMark({ size = 26 }: { size?: number }) {
  return (
    <span className="brand-mark" style={{ width: size, height: size }} aria-hidden>
      <span className="brand-ring" />
      <span className="brand-core" />
      <span className="brand-sat" style={{ left: "44.6%", top: "14.6%" }} />
      <span className="brand-sat" style={{ left: "68.8%", top: "57.7%" }} />
      <span className="brand-sat" style={{ left: "20%", top: "57.7%" }} />
    </span>
  );
}
