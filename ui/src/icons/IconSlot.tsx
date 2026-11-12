// The box an icon is drawn in beside a name: one icon wide and one line of its text tall, so the
// icon sits centred on the text's first line however many lines the text wraps to. The icon only
// repeats what the name says, so the box is hidden from screen readers and takes no pointer
// events (the icon's own title would otherwise show as a second tooltip). An empty box keeps the
// names of a list lined up when one of its rows has no icon.
import type { ReactElement } from 'react';
import { cn } from '../lib/cn';
import type { BrandIcon } from './icon';

/** The icon `icon` beside a name, or the empty box when there is none. */
export function IconSlot({
  icon,
  className,
}: {
  icon: BrandIcon | null;
  className?: string;
}): ReactElement {
  return (
    <span className={cn('icon-slot', className)} aria-hidden="true" data-icon={icon?.name}>
      {icon === null ? null : <icon.Svg />}
    </span>
  );
}
