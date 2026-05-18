import { useRef, useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { cn } from '@/lib/utils';
import { copyToClipboard } from '@/lib/clipboard';
import { fireConfettiFromElement } from '@/lib/confetti';

const methods = [
  { id: 'npx', label: 'npx', code: 'npx @1337xcode/cortex install' },
  {
    id: 'shell',
    label: 'Shell',
    code: 'curl -fsSL https://raw.githubusercontent.com/1337Xcode/cortex/main/install.sh | sh',
  },
  {
    id: 'source',
    label: 'Source',
    code: 'git clone https://github.com/1337Xcode/cortex && cd cortex && cargo build --release',
  },
] as const;

export default function InstallTabs() {
  const [active, setActive] = useState<(typeof methods)[number]['id']>('npx');
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const cardRef = useRef<HTMLDivElement>(null);
  const method = methods.find((m) => m.id === active) ?? methods[0];

  async function copy() {
    setCopyError(false);
    const ok = await copyToClipboard(method.code);
    if (!ok) {
      setCopyError(true);
      setTimeout(() => setCopyError(false), 2500);
      return;
    }
    setCopied(true);
    // Fire confetti from behind the card (card center)
    if (cardRef.current) {
      fireConfettiFromElement(cardRef.current, 150);
    }
    setTimeout(() => setCopied(false), 2000);
  }

  return (
    <div ref={cardRef} className="relative rounded-xl border border-border bg-card shadow-lg overflow-hidden">
      {/* Subtle gradient top accent */}
      <div className="absolute inset-x-0 top-0 h-px bg-gradient-to-r from-transparent via-foreground/20 to-transparent" />

      <div className="flex border-b border-border bg-muted/30" role="tablist" aria-label="Install method">
        {methods.map((m) => (
          <button
            key={m.id}
            type="button"
            role="tab"
            aria-selected={active === m.id}
            onClick={() => setActive(m.id)}
            className={cn(
              'flex-1 px-4 py-3 text-sm font-medium transition-colors border-b-2 -mb-px',
              active === m.id
                ? 'border-foreground text-foreground bg-card'
                : 'border-transparent text-muted-foreground hover:text-foreground hover:bg-muted/40',
            )}
          >
            {m.label}
          </button>
        ))}
      </div>

      <div role="tabpanel" className="flex items-center gap-3 p-4 sm:p-5 min-h-[5rem] bg-card">
        <div className="flex-1 min-w-0 rounded-lg bg-muted/40 border border-border/50 px-4 py-3">
          <pre className="overflow-x-auto font-mono text-sm sm:text-[15px] text-foreground whitespace-nowrap">
            <code className="select-all">{method.code}</code>
          </pre>
        </div>
        <button
          type="button"
          onClick={() => void copy()}
          className={cn(
            'inline-flex shrink-0 items-center gap-1.5 rounded-lg px-4 py-2.5 text-sm font-medium transition-all duration-200',
            copied
              ? 'bg-emerald-500/10 border border-emerald-500/30 text-emerald-600'
              : 'border border-border bg-muted/50 text-foreground hover:bg-muted hover:scale-105',
          )}
          aria-label={copied ? 'Copied' : copyError ? 'Copy failed' : 'Copy command'}
        >
          {copied ? <Check className="size-4" /> : <Copy className="size-4" />}
          <span className="hidden sm:inline">
            {copied ? 'Copied!' : copyError ? 'Failed' : 'Copy'}
          </span>
        </button>
      </div>
    </div>
  );
}
