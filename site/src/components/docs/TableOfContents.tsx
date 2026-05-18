import { useEffect, useRef, useState } from 'react';
import { cn } from '@/lib/utils';

export interface TocHeading {
  depth: number;
  slug: string;
  text: string;
}

type Props = {
  headings: TocHeading[];
};

export default function TableOfContents({ headings }: Props) {
  const tocHeadings = headings.filter((h) => h.depth >= 2 && h.depth <= 3);
  const listRef = useRef<HTMLUListElement>(null);
  const [activeSlug, setActiveSlug] = useState<string | null>(null);

  useEffect(() => {
    if (tocHeadings.length === 0) return;

    const headingEls = tocHeadings
      .map((h) => document.getElementById(h.slug))
      .filter(Boolean) as HTMLElement[];

    if (headingEls.length === 0) return;

    const offset = 112;

    const updateActive = () => {
      let current = headingEls[0]?.id ?? null;
      for (const el of headingEls) {
        if (el.getBoundingClientRect().top <= offset) {
          current = el.id;
        }
      }
      if (current) setActiveSlug(current);
    };

    updateActive();
    window.addEventListener('scroll', updateActive, { passive: true });
    window.addEventListener('resize', updateActive);
    return () => {
      window.removeEventListener('scroll', updateActive);
      window.removeEventListener('resize', updateActive);
    };
  }, [tocHeadings]);

  useEffect(() => {
    if (!activeSlug || !listRef.current) return;
    const activeLink = listRef.current.querySelector<HTMLAnchorElement>(
      `[data-toc-link="${activeSlug}"]`,
    );
    if (!activeLink) return;
    activeLink.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [activeSlug]);

  if (tocHeadings.length === 0) return null;

  return (
    <nav aria-label="Table of contents" className="flex flex-col h-full max-h-[calc(100vh-7rem)]" id="toc-nav">
      <h3 className="text-xs font-semibold uppercase tracking-wider text-muted-foreground mb-3 shrink-0">
        On this page
      </h3>
      <ul ref={listRef} className="space-y-1 text-sm overflow-y-auto flex-1 overscroll-contain pr-1 scroll-smooth">
        {tocHeadings.map((heading) => {
          const isActive = activeSlug === heading.slug;
          return (
            <li key={heading.slug} className={heading.depth === 3 ? 'pl-3' : ''}>
              <a
                href={`#${heading.slug}`}
                data-toc-link={heading.slug}
                className={cn(
                  'block py-1 transition-colors duration-200 border-l-2 pl-2',
                  isActive
                    ? 'text-foreground font-medium border-foreground'
                    : 'text-muted-foreground hover:text-foreground border-transparent',
                )}
              >
                {heading.text}
              </a>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
