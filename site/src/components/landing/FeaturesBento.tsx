import { useEffect, useRef, useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { BentoGrid, BentoGridItem } from '@/components/ui/bento-grid';
import { cn } from '@/lib/utils';
import {
  BinaryHint,
  CiGateStatic,
  FederationConsole,
  GitChurnStatic,
  IndexingMeter,
  LanguageMarqueeInteractive,
  McpToolGrid,
  MemorySymbols,
  SearchModeToggle,
  SecurityList,
} from './BentoInteractives';
import { BentoGraphAnimation } from './BentoGraphAnimation';

const languages = [
  'python', 'javascript', 'typescript', 'rust', 'go', 'java',
  'cplusplus', 'csharp', 'ruby', 'swift', 'kotlin', 'php',
];

type BentoCardProps = {
  className?: string;
  title: string;
  description?: string;
  stat?: string;
  statSuffix?: string;
  children?: React.ReactNode;
  delay?: number;
};

function BentoCard({ className, title, description, stat, statSuffix, children, delay = 0 }: BentoCardProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const obs = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          obs.disconnect();
        }
      },
      { threshold: 0.08 },
    );
    obs.observe(el);
    return () => obs.disconnect();
  }, []);

  return (
    <Card
      ref={ref}
      style={{ transitionDelay: `${delay}ms` }}
      className={cn(
        'bento-card flex h-full min-h-0 flex-col border-border bg-card p-0 shadow-sm transition-[opacity,transform] duration-500 ease-out',
        !visible && 'opacity-0 translate-y-3',
        visible && 'opacity-100 translate-y-0',
        className,
      )}
    >
      <CardHeader className="shrink-0 space-y-1.5 px-4 pt-4 pb-2 sm:px-5 sm:pt-4">
        <div className="flex items-start justify-between gap-3">
          <CardTitle className="text-base sm:text-lg font-semibold leading-tight">{title}</CardTitle>
          {stat && (
            <p className="text-right shrink-0 leading-none">
              <span className="text-xl sm:text-2xl font-bold tabular-nums">{stat}</span>
              {statSuffix && (
                <span className="block text-[10px] font-normal text-muted-foreground mt-0.5">{statSuffix}</span>
              )}
            </p>
          )}
        </div>
        {description && (
          <CardDescription className="text-xs sm:text-sm leading-snug">{description}</CardDescription>
        )}
      </CardHeader>
      {children && (
        <div className="bento-card-body flex min-h-0 flex-1 flex-col overflow-hidden px-4 pb-4 sm:px-5 sm:pb-4 pt-0">
          {children}
        </div>
      )}
    </Card>
  );
}

export default function FeaturesBento() {
  return (
    <section className="py-16 sm:py-24 bg-surface-sunken">
      <div className="mx-auto max-w-7xl px-4 sm:px-6 lg:px-8">
        <div className="mb-10 sm:mb-12 text-center max-w-3xl mx-auto">
          <h2 className="text-3xl sm:text-4xl md:text-5xl font-bold tracking-tight text-text">
            Key capabilities
          </h2>
          <p className="mt-4 text-lg sm:text-xl text-muted-foreground leading-relaxed">
            Local code intelligence built for AI agents.
          </p>
        </div>

        <BentoGrid className="lg:grid-cols-4">
          <BentoGridItem className="sm:col-span-2 lg:col-span-2 lg:row-span-2 min-h-[14rem] lg:min-h-0">
            <BentoCard className="h-full" title="Languages" description="Tree-sitter call graphs with incremental re-parse." stat="29" statSuffix="languages" delay={0}>
              <LanguageMarqueeInteractive languages={languages} />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="sm:col-span-2 lg:col-span-2 min-h-[12rem]">
            <BentoCard className="h-full" title="MCP tools" description="Navigation, graph, security, memory, federation. Smart ask routing picks the right tool." stat="32" statSuffix="5 in smart mode" delay={40}>
              <McpToolGrid />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="min-h-[11rem]">
            <BentoCard className="h-full" title="Indexing" description="Native file watchers. Incremental updates in milliseconds." delay={80}>
              <IndexingMeter />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="min-h-[11rem]">
            <BentoCard className="h-full" title="Security" description="Taint flow analysis, OWASP Top 10 patterns, SBOM export." delay={100}>
              <SecurityList />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="min-h-[11rem]">
            <BentoCard className="h-full" title="Memory" description="Cross-session context linked to symbols. Staleness when code changes." delay={120}>
              <MemorySymbols />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="sm:col-span-2 lg:col-span-2 min-h-[12rem]">
            <BentoCard className="h-full" title="Federation" description="Attach multiple repos into one queryable graph." delay={140}>
              <FederationConsole />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="min-h-[11rem]">
            <BentoCard className="h-full" title="Hybrid search" description="FTS5 BM25 plus optional local ONNX embeddings." delay={160}>
              <SearchModeToggle />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="min-h-[11rem]">
            <BentoCard className="h-full" title="CI gates" description="Configurable quality thresholds and exit codes for pipelines." delay={180}>
              <CiGateStatic />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="sm:col-span-2 lg:col-span-2 min-h-[12rem]">
            <BentoCard className="h-full" title="3D visualization" description="Interactive call-graph viewer from cortex viz." delay={200}>
              <BentoGraphAnimation />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="min-h-[11rem]">
            <BentoCard className="h-full" title="Single binary" description="No Node, Python, or Docker required." stat="~15" statSuffix="MB" delay={220}>
              <BinaryHint />
            </BentoCard>
          </BentoGridItem>

          <BentoGridItem className="sm:col-span-2 lg:col-span-4 min-h-[11rem]">
            <BentoCard className="h-full" title="Git intelligence" description="Hotspots, churn metrics, and branch call-graph diffs." delay={240}>
              <GitChurnStatic />
            </BentoCard>
          </BentoGridItem>
        </BentoGrid>
      </div>
    </section>
  );
}
