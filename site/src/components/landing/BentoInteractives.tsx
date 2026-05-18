import { useEffect, useRef, useState } from 'react';
import { Marquee } from '@/components/ui/marquee';
import { cn } from '@/lib/utils';

/** Counts match `get_tool_definitions()` in cortex/src/mcp/server.rs (32 total). */
const mcpTools = [
  { name: 'Structural', count: 10 },
  { name: 'Memory', count: 5 },
  { name: 'Security', count: 3 },
  { name: 'Search', count: 2 },
  { name: 'HTTP', count: 2 },
  { name: 'Analysis', count: 9 },
] as const;

export function McpToolGrid() {
  return (
    <div className="flex min-h-0 flex-1 flex-col justify-end pt-1">
      <ul className="grid grid-cols-2 gap-x-3 gap-y-2 sm:gap-y-2.5 text-[10px] sm:text-[11px] leading-tight text-muted-foreground">
        {mcpTools.map((t) => (
          <li key={t.name} className="flex items-center justify-between gap-2 tabular-nums min-w-0">
            <span className="truncate">{t.name}</span>
            <span className="font-semibold text-foreground shrink-0">{t.count}</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

export function IndexingMeter() {
  const [ms, setMs] = useState(0);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const obs = new IntersectionObserver(([e]) => {
      if (!e.isIntersecting) return;
      const start = performance.now();
      const tick = (now: number) => {
        const p = Math.min((now - start) / 1000, 1);
        setMs(Math.round((1 - (1 - p) ** 3) * 13));
        if (p < 1) requestAnimationFrame(tick);
      };
      requestAnimationFrame(tick);
      obs.disconnect();
    }, { threshold: 0.4 });
    obs.observe(el);
    return () => obs.disconnect();
  }, []);
  return (
    <div ref={ref} className="mt-auto pt-2">
      <p className="text-3xl font-bold tabular-nums">
        {ms}
        <span className="text-sm font-normal text-muted-foreground ml-1">ms</span>
      </p>
      <div className="mt-2 h-1.5 rounded-full bg-muted overflow-hidden">
        <div
          className="h-full rounded-full bg-blue-600 transition-[width] duration-300 ease-out"
          style={{ width: `${(ms / 13) * 100}%` }}
        />
      </div>
    </div>
  );
}

const securityFeatures = ['Taint flow analysis', 'OWASP Top 10', 'SBOM export'];

export function SecurityList() {
  return (
    <ul className="mt-3 space-y-1.5 text-xs sm:text-sm text-muted-foreground">
      {securityFeatures.map((item) => (
        <li key={item} className="flex items-center gap-2">
          <span className="size-1.5 shrink-0 rounded-full bg-red-500/80" aria-hidden />
          {item}
        </li>
      ))}
    </ul>
  );
}

export function SearchModeToggle() {
  const [mode, setMode] = useState<'fts' | 'vector'>('fts');
  const [paused, setPaused] = useState(false);

  useEffect(() => {
    if (paused) return;
    const id = window.setInterval(() => {
      setMode((m) => (m === 'fts' ? 'vector' : 'fts'));
    }, 2800);
    return () => window.clearInterval(id);
  }, [paused]);

  return (
    <div
      className="flex min-h-0 flex-1 flex-col justify-end pt-1"
      onMouseEnter={() => setPaused(true)}
      onMouseLeave={() => setPaused(false)}
    >
      <div className="relative inline-flex w-full max-w-[13rem] rounded-lg border border-border bg-muted/40 p-0.5">
        <span
          aria-hidden
          className={cn(
            'pointer-events-none absolute inset-y-0.5 w-[calc(50%-4px)] rounded-md bg-card shadow-sm transition-[left] duration-300 ease-out',
            mode === 'vector' ? 'left-[calc(50%+2px)]' : 'left-0.5',
          )}
        />
        {(['fts', 'vector'] as const).map((m) => (
          <button
            key={m}
            type="button"
            onClick={() => setMode(m)}
            className={cn(
              'relative z-10 flex-1 rounded-md px-3 py-1.5 text-xs font-medium transition-colors',
              mode === m ? 'text-foreground' : 'text-muted-foreground hover:text-foreground',
            )}
          >
            {m === 'fts' ? 'BM25' : 'Vectors'}
          </button>
        ))}
      </div>
      <p className="mt-2 text-[11px] sm:text-xs leading-snug text-muted-foreground">
        {mode === 'fts' ? 'Keyword + structural FTS5 search' : 'Local ONNX embeddings for semantic match'}
      </p>
    </div>
  );
}

const FEDERATION_SCRIPT = [
  { type: 'cmd' as const, text: 'cortex federate add ../auth-service' },
  { type: 'out' as const, text: "Added '../auth-service' to federation (1 repo)" },
  { type: 'cmd' as const, text: 'cortex federate add ../frontend' },
  { type: 'out' as const, text: "Added '../frontend' to federation (2 repos)" },
  { type: 'cmd' as const, text: 'cortex federate list' },
  { type: 'out' as const, text: '2 federated repos · cross-repo graph queries' },
];

export function FederationConsole() {
  const [lines, setLines] = useState<{ type: 'cmd' | 'out'; text: string }[]>([]);
  const [current, setCurrent] = useState('');
  const [lineIndex, setLineIndex] = useState(0);
  const [charIndex, setCharIndex] = useState(0);
  const [phase, setPhase] = useState<'typing' | 'pause' | 'done'>('typing');
  const ref = useRef<HTMLDivElement>(null);
  const [playing, setPlaying] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const obs = new IntersectionObserver(([e]) => {
      if (e.isIntersecting) setPlaying(true);
    }, { threshold: 0.3 });
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  useEffect(() => {
    if (!playing) return;
    if (phase === 'done') return;

    const line = FEDERATION_SCRIPT[lineIndex];
    if (!line) {
      setPhase('done');
      return;
    }

    if (charIndex < line.text.length) {
      const t = window.setTimeout(() => {
        setCurrent((c) => c + line.text[charIndex]);
        setCharIndex((i) => i + 1);
      }, line.type === 'cmd' ? 28 : 12);
      return () => window.clearTimeout(t);
    }

    const t = window.setTimeout(() => {
      setLines((prev) => [...prev, line]);
      setCurrent('');
      setCharIndex(0);
      if (lineIndex + 1 >= FEDERATION_SCRIPT.length) {
        setPhase('done');
      } else {
        setLineIndex((i) => i + 1);
      }
    }, line.type === 'cmd' ? 400 : 180);
    return () => window.clearTimeout(t);
  }, [charIndex, lineIndex, phase, playing]);

  const activeLine = FEDERATION_SCRIPT[lineIndex];
  const showCursor = phase !== 'done';

  return (
    <div
      ref={ref}
      className="flex h-full min-h-[7.5rem] flex-col overflow-hidden rounded-lg border border-border bg-[#1a1a1a] text-[#e8e6e3] font-mono text-[9px] sm:text-[10px] leading-tight shadow-inner"
    >
      <div className="flex shrink-0 items-center gap-1.5 border-b border-white/10 px-2 py-1 text-[8px] sm:text-[9px] text-white/50">
        <span className="size-1.5 rounded-full bg-red-500/80 sm:size-2" />
        <span className="size-1.5 rounded-full bg-amber-500/80 sm:size-2" />
        <span className="size-1.5 rounded-full bg-emerald-500/80 sm:size-2" />
        <span className="ml-1">cortex</span>
      </div>
      <div className="flex min-h-0 flex-1 flex-col justify-start overflow-y-auto p-2 sm:p-2.5 space-y-px [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {lines.map((line, i) => (
          <div key={i} className={line.type === 'cmd' ? 'text-cyan-400/95' : 'text-white/75 pl-0'}>
            {line.type === 'cmd' ? (
              <>
                <span className="text-emerald-500/90 select-none">$ </span>
                {line.text}
              </>
            ) : (
              line.text
            )}
          </div>
        ))}
        {activeLine && phase !== 'done' && (
          <div className={activeLine.type === 'cmd' ? 'text-cyan-400/95' : 'text-white/75'}>
            {activeLine.type === 'cmd' && <span className="text-emerald-500/90 select-none">$ </span>}
            {current}
            {showCursor && <span className="inline-block w-[6px] h-[12px] ml-0.5 bg-cyan-400/80 animate-pulse align-middle" />}
          </div>
        )}
      </div>
    </div>
  );
}

export function LanguageMarqueeInteractive({ languages }: { languages: string[] }) {
  return (
    <div
      className="mt-auto flex-1 min-h-[45%] flex items-end overflow-hidden mask-fade-x pt-2"
    >
      <Marquee pauseOnHover className="w-full [--duration:32s]">
        {languages.map((lang) => (
          <img
            key={lang}
            src={`https://cdn.jsdelivr.net/gh/devicons/devicon/icons/${lang}/${lang}-original.svg`}
            alt=""
            width={48}
            height={48}
            loading="lazy"
            decoding="async"
            className="mx-3 size-[clamp(2rem,6vw,3.25rem)] shrink-0 object-contain opacity-80"
          />
        ))}
      </Marquee>
    </div>
  );
}

export function MemorySymbols() {
  const symbols = ['fn authenticate', 'struct Session', 'write_adr'];
  return (
    <div className="mt-2 flex flex-wrap gap-1">
      {symbols.map((sym) => (
        <span
          key={sym}
          className="rounded-md border border-border bg-muted/30 px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground"
        >
          {sym}
        </span>
      ))}
    </div>
  );
}

export function CiGateStatic() {
  return (
    <div className="mt-3">
      <div className="flex justify-between text-xs text-muted-foreground mb-1">
        <span>Quality score</span>
        <span className="tabular-nums font-medium text-foreground">92%</span>
      </div>
      <div className="h-2 rounded-full bg-muted overflow-hidden">
        <div className="h-full w-[92%] rounded-full bg-emerald-600" />
      </div>
      <p className="mt-1.5 text-[10px] text-muted-foreground">Exit code 1 below threshold</p>
    </div>
  );
}

export function VizHint() {
  return (
    <p className="mt-3 font-mono text-[11px] sm:text-xs text-muted-foreground">
      <span className="text-foreground">cortex viz</span>
      <span className="mx-1.5 text-muted-foreground/60">→</span>
      standalone HTML call graph
    </p>
  );
}

export function BinaryHint() {
  return (
    <div className="mt-3 flex items-center gap-2">
      <span className="inline-flex size-10 items-center justify-center rounded-lg border border-border bg-muted/50 font-mono text-xs">
        .exe
      </span>
      <span className="text-xs text-muted-foreground">No runtime deps</span>
    </div>
  );
}

export function GitChurnStatic() {
  const files = [
    { name: 'src/index.rs', churn: 88 },
    { name: 'mcp/tools.rs', churn: 62 },
    { name: 'db/schema.sql', churn: 41 },
  ];
  return (
    <div className="mt-3 space-y-2">
      {files.map((f) => (
        <div key={f.name} className="space-y-0.5">
          <div className="flex justify-between text-[10px] sm:text-xs">
            <span className="font-mono truncate text-muted-foreground">{f.name}</span>
            <span className="tabular-nums text-muted-foreground">{f.churn}%</span>
          </div>
          <div className="h-1 rounded-full bg-muted overflow-hidden">
            <div className="h-full rounded-full bg-amber-600/70" style={{ width: `${f.churn}%` }} />
          </div>
        </div>
      ))}
    </div>
  );
}
