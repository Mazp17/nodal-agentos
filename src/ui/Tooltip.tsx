// shadcn/ui's Tooltip (Radix primitive, same API) with our own CSS instead of Tailwind.

import type { ComponentProps } from "react";
import { Tooltip as TooltipPrimitive } from "radix-ui";
import "./tooltip.css";

export function TooltipProvider({ delayDuration = 300, ...props }: ComponentProps<typeof TooltipPrimitive.Provider>) {
  return <TooltipPrimitive.Provider delayDuration={delayDuration} {...props} />;
}

// Not hoverable by default: tooltips here hold text only, and a portaled click would still
// bubble through React to the trigger's parent (e.g. open the card underneath).
export function Tooltip({ disableHoverableContent = true, ...props }: ComponentProps<typeof TooltipPrimitive.Root>) {
  return (
    <TooltipProvider>
      <TooltipPrimitive.Root disableHoverableContent={disableHoverableContent} {...props} />
    </TooltipProvider>
  );
}

export function TooltipTrigger(props: ComponentProps<typeof TooltipPrimitive.Trigger>) {
  return <TooltipPrimitive.Trigger {...props} />;
}

export function TooltipContent({
  className,
  sideOffset = 6,
  collisionPadding = 8,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        className={`tooltip ${className ?? ""}`}
        {...props}
      />
    </TooltipPrimitive.Portal>
  );
}
