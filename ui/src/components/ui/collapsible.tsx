// shadcn/ui's Collapsible on Radix: a trigger button with `aria-expanded` and `aria-controls`, and
// the content it shows and hides. The panels' sections build on it.
import { Collapsible as CollapsiblePrimitive } from 'radix-ui';
import type { ComponentProps, ReactElement } from 'react';

/** A region that opens and closes. */
export function Collapsible(props: ComponentProps<typeof CollapsiblePrimitive.Root>): ReactElement {
  return <CollapsiblePrimitive.Root data-slot="collapsible" {...props} />;
}

/** The button that opens and closes it. */
export function CollapsibleTrigger(
  props: ComponentProps<typeof CollapsiblePrimitive.CollapsibleTrigger>,
): ReactElement {
  return <CollapsiblePrimitive.CollapsibleTrigger data-slot="collapsible-trigger" {...props} />;
}

/** What it shows while open; not rendered while closed. */
export function CollapsibleContent(
  props: ComponentProps<typeof CollapsiblePrimitive.CollapsibleContent>,
): ReactElement {
  return <CollapsiblePrimitive.CollapsibleContent data-slot="collapsible-content" {...props} />;
}
