// The map's button, from shadcn/ui's Button, reworked onto the map's tokens: every size, gap,
// radius and colour is a token name the stylesheet defines, and the focus ring is the page's one
// ring (drawn by the stylesheet for every focused control). A native <button>; `asChild` lends the
// look to another element.
import { cva, type VariantProps } from 'class-variance-authority';
import { Slot } from 'radix-ui';
import type { ComponentProps, ReactElement } from 'react';
import { cn } from '../../lib/cn';

/** The button's classes by variant and size. */
export const buttonVariants = cva(
  'inline-flex shrink-0 cursor-pointer items-center justify-center gap-sm whitespace-nowrap rounded-control border border-transparent font-medium text-body transition-colors [&_svg]:pointer-events-none [&_svg]:size-icon [&_svg]:shrink-0',
  {
    variants: {
      variant: {
        /** The one filled button: the accent. */
        default: 'bg-primary text-primary-foreground',
        /** A control on the canvas: the surface, with its border. */
        outline: 'border-border bg-card text-foreground hover:bg-muted',
        /** A row in a list or a quiet action: no fill until pointed at. */
        ghost: 'text-foreground hover:bg-muted',
        /** Text that acts: the accent, underlined. */
        link: 'h-auto border-0 px-0 text-primary underline underline-offset-(--stroke-focus-offset)',
      },
      size: {
        default: 'h-control px-md',
        compact: 'h-control-compact px-sm text-small',
        icon: 'size-control',
        'icon-compact': 'size-control-compact',
        /** A list row: full width, text at the start. */
        row: 'h-row w-full justify-start px-sm font-normal',
        inline: 'h-auto px-0',
      },
    },
    defaultVariants: { variant: 'outline', size: 'default' },
  },
);

/** A button in one of the variants and sizes above. */
export function Button({
  className,
  variant = 'outline',
  size = 'default',
  asChild = false,
  type = 'button',
  ...props
}: ComponentProps<'button'> &
  VariantProps<typeof buttonVariants> & { readonly asChild?: boolean }): ReactElement {
  const Comp = asChild ? Slot.Root : 'button';
  return (
    <Comp
      data-slot="button"
      data-variant={variant}
      data-size={size}
      type={type}
      className={cn(buttonVariants({ variant, size, className }))}
      {...props}
    />
  );
}
