import { useCallback, useEffect, useRef, useState } from 'react';
import { Maximize2, Minimize2, RotateCcw } from 'lucide-react';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';

type Props = {
  src: string;
};

function IconAction({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <Tooltip>
      <TooltipTrigger
        type="button"
        onClick={onClick}
        className="inline-flex size-8 items-center justify-center rounded-lg text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
        aria-label={label}
        title={label}
      >
        {children}
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}

const VIZ_HINTS = 'Scroll to zoom · Drag to rotate · Click to inspect · / to search';

function postToViz(iframe: HTMLIFrameElement | null, data: unknown) {
  iframe?.contentWindow?.postMessage(data, '*');
}

export default function VizFrame({ src }: Props) {
  const wrapperRef = useRef<HTMLDivElement>(null);
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const [fullscreen, setFullscreen] = useState(false);

  const notifyViz = useCallback((active: boolean) => {
    postToViz(iframeRef.current, { type: 'cortex-viz-fullscreen', active });
    postToViz(iframeRef.current, { type: 'cortex-viz-resize' });
  }, []);

  const syncFullscreen = useCallback(() => {
    const doc = document as Document & { webkitFullscreenElement?: Element };
    const el = doc.fullscreenElement ?? doc.webkitFullscreenElement;
    const active = Boolean(el && wrapperRef.current && el === wrapperRef.current);
    setFullscreen(active);
    notifyViz(active);
  }, [notifyViz]);

  useEffect(() => {
    document.addEventListener('fullscreenchange', syncFullscreen);
    document.addEventListener('webkitfullscreenchange', syncFullscreen);
    return () => {
      document.removeEventListener('fullscreenchange', syncFullscreen);
      document.removeEventListener('webkitfullscreenchange', syncFullscreen);
    };
  }, [syncFullscreen]);

  useEffect(() => {
    const wrapper = wrapperRef.current;
    if (!wrapper) return;
    const ro = new ResizeObserver(() => {
      postToViz(iframeRef.current, { type: 'cortex-viz-resize' });
    });
    ro.observe(wrapper);
    return () => ro.disconnect();
  }, []);

  async function toggleFullscreen() {
    const el = wrapperRef.current;
    if (!el) return;
    const doc = document as Document & {
      webkitFullscreenElement?: Element;
      webkitExitFullscreen?: () => Promise<void>;
    };
    const active = doc.fullscreenElement ?? doc.webkitFullscreenElement;

    try {
      if (active) {
        await (document.exitFullscreen?.() ?? doc.webkitExitFullscreen?.());
      } else {
        const req =
          el.requestFullscreen?.bind(el) ??
          (
            el as HTMLElement & { webkitRequestFullscreen?: () => Promise<void> }
          ).webkitRequestFullscreen?.bind(el);
        await req?.();
      }
    } catch {
      /* user cancelled or browser blocked */
    }

    syncFullscreen();
  }

  function reload() {
    const iframe = iframeRef.current;
    if (iframe) iframe.src = src;
  }

  return (
    <div className="space-y-4">
      {!fullscreen && (
        <div className="flex flex-wrap items-center justify-between gap-3">
          <p className="text-sm text-muted-foreground max-w-xl">
            Interactive call graph from{' '}
            <code className="font-mono text-xs bg-muted px-1.5 py-0.5 rounded border border-border">
              cortex viz
            </code>
            . Click nodes, press{' '}
            <kbd className="text-[10px] bg-muted px-1 py-0.5 rounded border border-border">/</kbd> to
            search.
          </p>
          <div className="flex items-center gap-0.5">
            <IconAction label="Reload visualization" onClick={reload}>
              <RotateCcw className="size-[18px]" strokeWidth={1.75} aria-hidden />
            </IconAction>
            <IconAction label="Fullscreen" onClick={toggleFullscreen}>
              <Maximize2 className="size-[18px]" strokeWidth={1.75} aria-hidden />
            </IconAction>
          </div>
        </div>
      )}

      <div
        ref={wrapperRef}
        className={cn(
          'relative w-full overflow-hidden bg-card',
          fullscreen
            ? 'h-[100dvh] w-[100dvw] rounded-none border-0'
            : 'h-[min(80vh,720px)] min-h-[360px] rounded-xl border border-border shadow-sm',
        )}
      >
        <iframe
          ref={iframeRef}
          src={src}
          title="Cortex codebase visualization"
          className="absolute inset-0 z-0 h-full w-full border-0"
          allow="fullscreen"
          loading="lazy"
          onLoad={() => postToViz(iframeRef.current, { type: 'cortex-viz-resize' })}
        />
        {fullscreen && (
          <div className="pointer-events-none absolute bottom-4 left-4 z-50">
            <div className="pointer-events-auto flex items-center gap-0.5 rounded-lg border border-border/80 bg-background/95 p-0.5 shadow-md backdrop-blur-sm">
              <IconAction label="Reload visualization" onClick={reload}>
                <RotateCcw className="size-[18px]" strokeWidth={1.75} aria-hidden />
              </IconAction>
              <IconAction label="Exit fullscreen" onClick={toggleFullscreen}>
                <Minimize2 className="size-[18px]" strokeWidth={1.75} aria-hidden />
              </IconAction>
            </div>
          </div>
        )}
        {fullscreen ? (
          <div
            className="pointer-events-none absolute bottom-4 right-4 z-20 max-w-[min(100%,20rem)] rounded-lg border border-border/60 bg-background/90 px-3 py-2 text-[11px] leading-relaxed text-muted-foreground shadow-sm backdrop-blur-sm"
            aria-hidden
          >
            {VIZ_HINTS}
          </div>
        ) : null}
      </div>
    </div>
  );
}
