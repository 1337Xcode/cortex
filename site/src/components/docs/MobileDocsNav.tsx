import { useState, useEffect, useCallback } from 'react';
import { IconMenu2, IconX, IconList } from '@tabler/icons-react';

export interface DocNavItem {
  id: string;
  title: string;
  category: string;
}

export interface TocHeading {
  depth: number;
  slug: string;
  text: string;
}

interface Props {
  items: DocNavItem[];
  currentSlug: string;
  headings?: TocHeading[];
}

const BASE = import.meta.env.BASE_URL || '/';
const basePath = BASE.endsWith('/') ? BASE.slice(0, -1) : BASE;
const prefix = basePath && basePath !== '/' ? basePath : '';

const categoryLabels: Record<string, string> = {
  guides: 'Guides',
  reference: 'Reference',
  concepts: 'Concepts',
  general: 'General',
  General: 'General',
};

function docPath(id: string) {
  return `${prefix}/docs/${id}/`;
}

export default function MobileDocsNav({ items, currentSlug, headings = [] }: Props) {
  const [open, setOpen] = useState<'nav' | 'toc' | null>(null);
  const close = useCallback(() => setOpen(null), []);

  useEffect(() => {
    document.body.style.overflow = open ? 'hidden' : '';
    return () => {
      document.body.style.overflow = '';
    };
  }, [open]);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (e.key === 'Escape') close();
    }
    if (open) document.addEventListener('keydown', onKey);
    return () => document.removeEventListener('keydown', onKey);
  }, [open, close]);

  const categories = items.reduce<Record<string, DocNavItem[]>>((acc, item) => {
    const cat = item.category || 'General';
    if (!acc[cat]) acc[cat] = [];
    acc[cat].push(item);
    return acc;
  }, {});

  return (
    <>
      <div
        className="lg:hidden fixed bottom-4 left-4 z-40 flex flex-col items-stretch gap-2 pb-[env(safe-area-inset-bottom)]"
        aria-label="Mobile documentation shortcuts"
      >
        <button
          type="button"
          onClick={() => setOpen('nav')}
          className="inline-flex items-center gap-2 whitespace-nowrap rounded-lg border border-border bg-card px-3 py-2 text-sm font-medium text-foreground shadow-md hover:bg-muted"
        >
          <IconMenu2 size={16} stroke={1.5} aria-hidden />
          Docs
        </button>
        {headings.length > 0 && (
          <button
            type="button"
            onClick={() => setOpen('toc')}
            className="inline-flex items-center gap-2 whitespace-nowrap rounded-lg border border-border bg-card px-3 py-2 text-sm font-medium text-foreground shadow-md hover:bg-muted"
          >
            <IconList size={16} stroke={1.5} aria-hidden />
            On this page
          </button>
        )}
      </div>

      {open && (
        <div className="fixed inset-0 z-50 bg-black/40 backdrop-blur-sm lg:hidden" onClick={close} aria-hidden />
      )}

      <div
        className={`fixed top-0 left-0 z-[60] h-full w-[min(100%,18rem)] bg-surface-raised border-r border-border shadow-xl transform transition-transform duration-300 lg:hidden ${
          open === 'nav' ? 'translate-x-0' : '-translate-x-full pointer-events-none'
        }`}
        role="dialog"
        aria-modal={open === 'nav'}
        aria-label="Documentation navigation"
      >
        <div className="flex h-14 items-center justify-between border-b border-border px-4">
          <span className="font-semibold text-text text-sm">Documentation</span>
          <button type="button" onClick={close} className="p-2 text-text-muted" aria-label="Close">
            <IconX size={18} />
          </button>
        </div>
        <nav className="overflow-y-auto max-h-[calc(100vh-3.5rem)] p-4 space-y-6">
          {Object.entries(categories).map(([category, entries]) => (
            <div key={category}>
              <p className="mb-2 px-3 text-[11px] font-semibold uppercase tracking-wider text-muted-foreground">
                {categoryLabels[category] ?? category}
              </p>
              <ul className="space-y-0.5">
                {entries.map((entry) => (
                  <li key={entry.id}>
                    <a
                      href={docPath(entry.id)}
                      className={`block rounded-md px-3 py-2 text-sm transition-colors ${
                        entry.id === currentSlug
                          ? 'bg-muted font-medium text-foreground'
                          : 'text-muted-foreground hover:bg-muted/70 hover:text-foreground'
                      }`}
                      onClick={close}
                    >
                      {entry.title}
                    </a>
                  </li>
                ))}
              </ul>
            </div>
          ))}
        </nav>
      </div>

      <div
        className={`fixed top-0 right-0 z-[60] h-full w-[min(100%,16rem)] bg-surface-raised border-l border-border shadow-xl transform transition-transform duration-300 lg:hidden ${
          open === 'toc' ? 'translate-x-0' : 'translate-x-full pointer-events-none'
        }`}
        role="dialog"
        aria-modal={open === 'toc'}
        aria-label="Table of contents"
      >
        <div className="flex h-14 items-center justify-between border-b border-border px-4">
          <span className="font-semibold text-text text-sm">On this page</span>
          <button type="button" onClick={close} className="p-2 text-text-muted" aria-label="Close">
            <IconX size={18} />
          </button>
        </div>
        <nav className="p-4 overflow-y-auto max-h-[calc(100vh-3.5rem)]">
          <ul className="space-y-2 text-sm">
            {headings.filter((h) => h.depth <= 3).map((h) => (
              <li key={h.slug} style={{ paddingLeft: `${(h.depth - 2) * 12}px` }}>
                <a
                  href={`#${h.slug}`}
                  className="text-text-muted hover:text-text block py-0.5"
                  onClick={close}
                >
                  {h.text}
                </a>
              </li>
            ))}
          </ul>
        </nav>
      </div>
    </>
  );
}
