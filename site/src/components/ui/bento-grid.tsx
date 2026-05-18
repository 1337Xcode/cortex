import { type ComponentPropsWithoutRef, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

interface BentoGridProps extends ComponentPropsWithoutRef<'div'> {
  children: ReactNode;
  className?: string;
}

export function BentoGrid({ children, className, ...props }: BentoGridProps) {
  return (
    <div
      className={cn(
        'grid w-full auto-rows-[minmax(11rem,auto)] grid-cols-1 gap-3 sm:grid-cols-2 sm:gap-4 lg:grid-cols-3 lg:gap-4',
        className,
      )}
      {...props}
    >
      {children}
    </div>
  );
}

interface BentoGridItemProps extends ComponentPropsWithoutRef<'div'> {
  className?: string;
  children: ReactNode;
}

export function BentoGridItem({ className, children, ...props }: BentoGridItemProps) {
  return (
    <div className={cn('min-h-[11rem] min-w-0', className)} {...props}>
      {children}
    </div>
  );
}
