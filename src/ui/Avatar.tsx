import { avatarHue, initials } from "../lib/format";
import "./avatar.css";

/** Avatar con iniciales y matiz estable por nombre; vacío (punteado) sin nombre. */
export function Avatar({ name, size = "sm" }: { name: string | null | undefined; size?: "sm" | "md" }) {
  if (!name) return <span className={`avatar avatar-${size} avatar-empty`} title="Unassigned" />;
  return (
    <span className={`avatar avatar-${size}`} style={{ ["--hue" as string]: avatarHue(name) }} title={name}>
      {initials(name)}
    </span>
  );
}
