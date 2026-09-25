import { avatarHue, initials } from "../lib/format";
import "./avatar.css";

/** Avatar with initials and a stable hue per name; empty (dashed) without a name. */
export function Avatar({ name, size = "sm" }: { name: string | null | undefined; size?: "sm" | "md" }) {
  if (!name) return <span className={`avatar avatar-${size} avatar-empty`} title="Unassigned" />;
  return (
    <span className={`avatar avatar-${size}`} style={{ ["--hue" as string]: avatarHue(name) }} title={name}>
      {initials(name)}
    </span>
  );
}
