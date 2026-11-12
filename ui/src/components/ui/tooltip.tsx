// The map's tooltip, from shadcn/ui's Tooltip on Radix: it opens when the pointer rests on its
// trigger and when the trigger takes keyboard focus, and closes on Escape or when either leaves.
// Reworked onto the map's tokens: the text is small type on the inverted surface, with a fade that
// lasts the motion token (none under reduced motion).
import { Tooltip as TooltipPrimitive } from 'radix-ui';
import type { ComponentProps, ReactElement } from 'react';
import { cn } from '../../lib/cn';
import { SPACE, TOOLTIP_DELAY_MS } from '../../theme/tokens';

/** Shares the open delay among every tooltip of the page; mount it once around the app. */
export function TooltipProvider({
  delayDuration = TOOLTIP_DELAY_MS,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Provider>): ReactElement {
  return <TooltipPrimitive.Provider delayDuration={delayDuration} {...props} />;
}

/** One tooltip: a trigger and its content. */
export function Tooltip(props: ComponentProps<typeof TooltipPrimitive.Root>): ReactElement {
  return <TooltipPrimitive.Root data-slot="tooltip" {...props} />;
}

/** What the tooltip explains; with `asChild`, the child element itself. */
export function TooltipTrigger(
  props: ComponentProps<typeof TooltipPrimitive.Trigger>,
): ReactElement {
  return <TooltipPrimitive.Trigger data-slot="tooltip-trigger" {...props} />;
}

/** The explanation, in a portal above everything, kept inside the viewport. */
export function TooltipContent({
  className,
  sideOffset = SPACE.xs,
  collisionPadding = SPACE.sm,
  children,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Content>): ReactElement {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        collisionPadding={collisionPadding}
        className={cn(
          'tooltip z-50 w-fit max-w-(--size-intro) rounded-small bg-foreground px-sm py-xs text-small text-background break-words duration-(--motion-duration) data-[state=delayed-open]:animate-in data-[state=delayed-open]:fade-in-0 data-[state=closed]:animate-out data-[state=closed]:fade-out-0',
          className,
        )}
        {...props}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}
