import { useEffect, useState } from 'react';
import { Moon, Sun } from 'lucide-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

function readDark(): boolean {
  if (typeof document === 'undefined') return false;
  const stored = localStorage.getItem('theme');
  const prefersDark = globalThis.matchMedia('(prefers-color-scheme: dark)').matches;
  return stored === 'dark' || (!stored && prefersDark);
}

export default function ThemeToggle() {
  const [isDark, setIsDark] = useState(readDark);

  useEffect(() => {
    setIsDark(readDark());
    document.documentElement.classList.toggle('dark', readDark());
  }, []);

  function toggle() {
    const newDark = !isDark;
    setIsDark(newDark);
    document.documentElement.classList.toggle('dark', newDark);
    localStorage.setItem('theme', newDark ? 'dark' : 'light');
  }

  return (
    <Tooltip>
      <TooltipTrigger
        type="button"
        onClick={toggle}
        className="relative inline-flex size-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={isDark ? 'Switch to light mode' : 'Switch to dark mode'}
        title={isDark ? 'Light mode' : 'Dark mode'}
      >
        <Sun
          className={cn(
            'size-[18px] transition-opacity duration-200',
            isDark ? 'opacity-0 absolute' : 'opacity-100',
          )}
          strokeWidth={1.75}
          aria-hidden
        />
        <Moon
          className={cn(
            'size-[18px] transition-opacity duration-200',
            isDark ? 'opacity-100' : 'opacity-0 absolute',
          )}
          strokeWidth={1.75}
          aria-hidden
        />
      </TooltipTrigger>
      <TooltipContent side="bottom">{isDark ? 'Light mode' : 'Dark mode'}</TooltipContent>
    </Tooltip>
  );
}
