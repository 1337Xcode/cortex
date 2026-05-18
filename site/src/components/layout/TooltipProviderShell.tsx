import type { ReactNode } from 'react';
import { TooltipProvider } from '@/components/ui/tooltip';

export default function TooltipProviderShell({ children }: { children: ReactNode }) {
  return <TooltipProvider delay={200}>{children}</TooltipProvider>;
}
