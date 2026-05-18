import type { ReactNode } from 'react';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

type IconTooltipLinkProps = {
  href: string;
  label: string;
  children: ReactNode;
  external?: boolean;
  className?: string;
};

export function IconTooltipLink({ href, label, children, external, className }: IconTooltipLinkProps) {
  return (
    <Tooltip>
      <TooltipTrigger
        className={cn(
          'inline-flex size-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring',
          className,
        )}
        render={
          <a
            href={href}
            aria-label={label}
            title={label}
            {...(external ? { target: '_blank', rel: 'noopener noreferrer' } : {})}
          />
        }
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}
