import { useEffect, useRef, useState, type ReactNode } from 'react';
import { cn } from '@/lib/utils';

interface ScrollHeaderProps {
  children: ReactNode;
}

export default function ScrollHeader({ children }: ScrollHeaderProps) {
  const [visible, setVisible] = useState(true);
  const lastY = useRef(0);
  const ticking = useRef(false);

  useEffect(() => {
    const onScroll = () => {
      if (ticking.current) return;
      ticking.current = true;
      requestAnimationFrame(() => {
        const y = window.scrollY;
        if (y <= 8) {
          setVisible(true);
        } else if (y > lastY.current + 8) {
          setVisible(false);
        } else if (y < lastY.current - 8) {
          setVisible(true);
        }
        lastY.current = y;
        ticking.current = false;
      });
    };
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => window.removeEventListener('scroll', onScroll);
  }, []);

  return (
    <header
      className={cn(
        'fixed top-0 z-50 w-full border-b border-border bg-surface-raised/90 backdrop-blur-md transition-transform duration-300 ease-out',
        visible ? 'translate-y-0' : '-translate-y-full',
      )}
    >
      {children}
    </header>
  );
}
