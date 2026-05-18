import { useState, useEffect, useCallback, useRef } from 'react';
import { createPortal } from 'react-dom';
import { IconMenu2, IconX, IconBook, IconView360, IconBug, IconBrandGithub, IconBrandNpm } from '@tabler/icons-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';

const BASE = import.meta.env.BASE_URL || '/';
const basePath = BASE.endsWith('/') ? BASE.slice(0, -1) : BASE;
const prefix = basePath && basePath !== '/' ? basePath : '';

function path(p: string) {
  return `${prefix}${p.startsWith('/') ? p : `/${p}`}`;
}

const links = [
  { href: path('/docs/getting-started/'), label: 'Docs', icon: IconBook },
  { href: path('/visualization/'), label: 'Visualization', icon: IconView360 },
  { href: path('/issues/'), label: 'Issues', icon: IconBug },
  { href: 'https://github.com/1337Xcode/cortex', label: 'GitHub', icon: IconBrandGithub, external: true },
  { href: 'https://www.npmjs.com/package/@1337xcode/cortex', label: 'npm', icon: IconBrandNpm, external: true },
];

function MobileNavPanel({
  isOpen,
  close,
  panelRef,
}: {
  isOpen: boolean;
  close: () => void;
  panelRef: React.RefObject<HTMLDivElement | null>;
}) {
  if (!isOpen) return null;

  return createPortal(
    <>
      <div
        className="fixed inset-0 z-[80] bg-black/55"
        onClick={close}
        aria-hidden="true"
      />
      <div
        id="mobile-nav-panel"
        ref={panelRef}
        className="fixed top-0 right-0 z-[90] flex h-full w-[min(100%,20rem)] flex-col border-l border-border bg-card shadow-2xl"
        role="dialog"
        aria-modal="true"
        aria-label="Mobile navigation"
      >
        <div className="flex h-14 shrink-0 items-center justify-between border-b border-border bg-card px-4">
          <span className="text-base font-semibold text-text">Menu</span>
          <button
            type="button"
            onClick={close}
            className="rounded-lg p-2 text-text-muted hover:bg-muted hover:text-text"
            aria-label="Close menu"
          >
            <IconX size={20} stroke={1.5} />
          </button>
        </div>
        <nav className="flex flex-1 flex-col gap-1 overflow-y-auto bg-card p-4">
          {links.map((link) => {
            const Icon = link.icon;
            return (
              <a
                key={link.href}
                href={link.href}
                className="flex items-center gap-3 rounded-lg px-3 py-3 text-sm font-medium text-text-muted transition-colors hover:bg-muted hover:text-text"
                {...(link.external ? { target: '_blank', rel: 'noopener noreferrer' } : {})}
                onClick={close}
              >
                <Icon size={18} stroke={1.5} className="shrink-0 opacity-70" />
                {link.label}
              </a>
            );
          })}
        </nav>
      </div>
    </>,
    document.body,
  );
}

export default function MobileNav() {
  const [isOpen, setIsOpen] = useState(false);
  const [mounted, setMounted] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);

  const close = useCallback(() => setIsOpen(false), []);

  useEffect(() => {
    setMounted(true);
  }, []);

  useEffect(() => {
    if (!isOpen) {
      document.body.style.overflow = '';
      return;
    }
    document.body.style.overflow = 'hidden';
    const firstLink = panelRef.current?.querySelector<HTMLElement>('a');
    firstLink?.focus();

    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') close();
    }
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('keydown', onKey);
      document.body.style.overflow = '';
    };
  }, [isOpen, close]);

  return (
    <>
      <Tooltip>
        <TooltipTrigger
          type="button"
          onClick={() => setIsOpen((o) => !o)}
          className="rounded-lg p-2 text-text-muted transition-colors hover:bg-muted hover:text-text"
          aria-label={isOpen ? 'Close menu' : 'Open menu'}
          title={isOpen ? 'Close menu' : 'Open menu'}
          aria-expanded={isOpen}
          aria-controls="mobile-nav-panel"
        >
          {isOpen ? <IconX size={22} stroke={1.5} /> : <IconMenu2 size={22} stroke={1.5} />}
        </TooltipTrigger>
        <TooltipContent side="bottom">{isOpen ? 'Close menu' : 'Open menu'}</TooltipContent>
      </Tooltip>

      {mounted ? <MobileNavPanel isOpen={isOpen} close={close} panelRef={panelRef} /> : null}
    </>
  );
}
