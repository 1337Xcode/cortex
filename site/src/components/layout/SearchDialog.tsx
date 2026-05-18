import { useState, useEffect, useRef, useCallback } from 'react';
import { createPortal } from 'react-dom';
import { Search } from 'lucide-react';
import Fuse from 'fuse.js';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

interface SearchItem {
  title: string;
  description: string;
  slug: string;
  category?: string;
}

const searchIndex: SearchItem[] = [
  { title: 'Getting Started', description: 'Install Cortex and index your first repository in under a minute.', slug: 'getting-started', category: 'general' },
  { title: 'Architecture', description: 'How Cortex works internally: tree-sitter parsing, SQLite storage, and MCP server.', slug: 'architecture', category: 'general' },
  { title: 'CLI Reference', description: 'Complete reference for all Cortex CLI commands and options.', slug: 'cli-reference', category: 'general' },
  { title: 'Configuration', description: 'Configure Cortex behavior with cortex.toml and environment variables.', slug: 'configuration', category: 'general' },
  { title: 'IDE Setup', description: 'Set up Cortex with VS Code, Cursor, Windsurf, and other editors.', slug: 'ide-setup', category: 'general' },
  { title: 'MCP Tools', description: 'Reference for all 32 MCP tools exposed by the Cortex server.', slug: 'tools', category: 'general' },
];

const fuse = new Fuse(searchIndex, {
  keys: ['title', 'description'],
  threshold: 0.3,
  includeScore: true,
});

function getPlatform(): 'mac' | 'windows' | 'mobile' {
  if (typeof navigator === 'undefined') return 'windows';
  const ua = navigator.userAgent;
  if (/iPhone|iPad|iPod|Android/i.test(ua)) return 'mobile';
  if (/Mac/i.test(navigator.platform || ua)) return 'mac';
  return 'windows';
}

function navigateToDoc(slug: string) {
  const base = import.meta.env.BASE_URL || '/';
  const basePath = base.endsWith('/') ? base.slice(0, -1) : base;
  const prefix = basePath && basePath !== '/' ? basePath : '';
  globalThis.location.href = `${prefix}/docs/${slug}/`;
}

function SearchOverlay({
  query,
  setQuery,
  results,
  selectedIndex,
  setSelectedIndex,
  close,
  inputRef,
  listRef,
}: {
  query: string;
  setQuery: (q: string) => void;
  results: SearchItem[];
  selectedIndex: number;
  setSelectedIndex: (fn: (i: number) => number) => void;
  close: () => void;
  inputRef: React.RefObject<HTMLInputElement | null>;
  listRef: React.RefObject<HTMLUListElement | null>;
}) {
  return (
    <div
      className="fixed inset-0 z-[9999] flex items-start justify-center px-4 pt-[12vh] sm:pt-[18vh]"
      role="dialog"
      aria-modal="true"
      aria-label="Search documentation"
    >
      <div
        role="presentation"
        className="absolute inset-0 cursor-default"
        style={{
          backgroundColor: 'color-mix(in srgb, var(--color-surface) 72%, transparent)',
          backdropFilter: 'blur(24px) saturate(1.2)',
          WebkitBackdropFilter: 'blur(24px) saturate(1.2)',
        }}
        onClick={close}
        onKeyDown={(e) => e.key === 'Escape' && close()}
      />

      <div
        className="relative z-10 w-full max-w-lg overflow-hidden rounded-xl border border-border bg-card shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center gap-3 border-b border-border px-4 py-3">
          <Search className="size-[18px] shrink-0 text-muted-foreground" strokeWidth={1.75} aria-hidden />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search documentation..."
            className="flex-1 bg-transparent text-sm text-foreground placeholder:text-muted-foreground outline-none"
            aria-label="Search documentation"
            aria-controls="search-results-list"
            aria-activedescendant={
              results[selectedIndex] ? `search-result-${results[selectedIndex].slug}` : undefined
            }
          />
          <kbd className="shrink-0 rounded border border-border bg-muted px-1.5 py-0.5 text-[10px] font-mono text-muted-foreground">
            ESC
          </kbd>
        </div>

        <div className="max-h-[min(50vh,20rem)] overflow-y-auto p-2" id="search-results-list">
          {results.length === 0 ? (
            <p className="px-4 py-8 text-center text-sm text-muted-foreground">
              No results found for &ldquo;{query}&rdquo;
            </p>
          ) : (
            <ul ref={listRef} role="listbox">
              {results.map((item, index) => {
                const active = index === selectedIndex;
                return (
                  <li key={item.slug} role="option" aria-selected={active}>
                    <button
                      id={`search-result-${item.slug}`}
                      type="button"
                      data-search-active={active ? 'true' : undefined}
                      onClick={() => {
                        close();
                        navigateToDoc(item.slug);
                      }}
                      onMouseEnter={() => setSelectedIndex(() => index)}
                      className={cn(
                        'w-full rounded-lg px-3 py-2.5 text-left transition-colors',
                        active ? 'bg-muted ring-1 ring-border' : 'hover:bg-muted/60',
                      )}
                    >
                      <div className="text-sm font-medium text-foreground">{item.title}</div>
                      <div className="mt-0.5 line-clamp-1 text-xs text-muted-foreground">{item.description}</div>
                    </button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>

        <div className="flex items-center gap-4 border-t border-border px-4 py-2 text-[10px] text-muted-foreground">
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border bg-muted px-1 py-0.5">↑↓</kbd> navigate
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border bg-muted px-1 py-0.5">↵</kbd> select
          </span>
          <span className="flex items-center gap-1">
            <kbd className="rounded border border-border bg-muted px-1 py-0.5">esc</kbd> close
          </span>
        </div>
      </div>
    </div>
  );
}

export default function SearchDialog() {
  const [isOpen, setIsOpen] = useState(false);
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<SearchItem[]>([]);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const [platform, setPlatform] = useState<'mac' | 'windows' | 'mobile'>('windows');
  const [mounted, setMounted] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLUListElement>(null);

  useEffect(() => {
    setMounted(true);
    setPlatform(getPlatform());
  }, []);

  const open = useCallback(() => {
    setIsOpen(true);
    setQuery('');
    setResults(searchIndex);
    setSelectedIndex(0);
    document.body.style.overflow = 'hidden';
    document.documentElement.setAttribute('data-search-open', '');
  }, []);

  const close = useCallback(() => {
    setIsOpen(false);
    setQuery('');
    document.body.style.overflow = '';
    document.documentElement.removeAttribute('data-search-open');
  }, []);

  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key === 'k') {
        e.preventDefault();
        if (isOpen) close();
        else open();
        return;
      }
      if (!isOpen) return;

      if (e.key === 'Escape') {
        e.preventDefault();
        close();
        return;
      }

      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, Math.max(results.length - 1, 0)));
        return;
      }

      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }

      if (e.key === 'Enter' && results[selectedIndex]) {
        e.preventDefault();
        const slug = results[selectedIndex].slug;
        close();
        navigateToDoc(slug);
      }
    }

    document.addEventListener('keydown', handleKeyDown);
    return () => document.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, open, close, results, selectedIndex]);

  useEffect(() => {
    if (isOpen && inputRef.current) {
      inputRef.current.focus();
    }
  }, [isOpen]);

  useEffect(() => {
    if (!query.trim()) {
      setResults(searchIndex);
      setSelectedIndex(0);
      return;
    }
    const fuseResults = fuse.search(query);
    setResults(fuseResults.map((r) => r.item));
    setSelectedIndex(0);
  }, [query]);

  useEffect(() => {
    if (!isOpen || !listRef.current) return;
    const active = listRef.current.querySelector<HTMLElement>('[data-search-active="true"]');
    active?.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  }, [selectedIndex, isOpen, results]);

  useEffect(() => {
    return () => {
      document.body.style.overflow = '';
      document.documentElement.removeAttribute('data-search-open');
    };
  }, []);

  const shortcutLabel = platform === 'mac' ? '⌘K' : platform === 'windows' ? 'Ctrl+K' : null;

  return (
    <>
      <Tooltip>
        <TooltipTrigger
          type="button"
          onClick={open}
          className="flex items-center gap-2 px-2 py-1.5 sm:px-3 text-sm text-muted-foreground bg-muted/50 border border-border rounded-lg hover:bg-muted transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
          aria-label="Search documentation"
          aria-expanded={isOpen}
          title="Search documentation"
        >
          <Search className="size-[18px] shrink-0" strokeWidth={1.75} aria-hidden />
          <span className="hidden sm:inline">Search</span>
          {shortcutLabel && (
            <kbd className="hidden sm:inline-flex items-center gap-0.5 px-1.5 py-0.5 text-[10px] font-mono bg-card rounded border border-border text-muted-foreground">
              {shortcutLabel}
            </kbd>
          )}
        </TooltipTrigger>
        <TooltipContent side="bottom">
          Search documentation{shortcutLabel ? ` (${shortcutLabel})` : ''}
        </TooltipContent>
      </Tooltip>

      {mounted && isOpen
        ? createPortal(
            <SearchOverlay
              query={query}
              setQuery={setQuery}
              results={results}
              selectedIndex={selectedIndex}
              setSelectedIndex={setSelectedIndex}
              close={close}
              inputRef={inputRef}
              listRef={listRef}
            />,
            document.body,
          )
        : null}
    </>
  );
}
