// shadcn/ui's Kbd (same API) with our own CSS instead of Tailwind.

import type { ComponentProps } from "react";
import "./kbd.css";

export function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return <kbd data-slot="kbd" className={`kbd ${className ?? ""}`} {...props} />;
}

export function KbdGroup({ className, ...props }: ComponentProps<"kbd">) {
  return <kbd data-slot="kbd-group" className={`kbd-group ${className ?? ""}`} {...props} />;
}
