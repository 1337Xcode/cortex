// The page: loads the snapshot and the summary, then shows the map with the search box and the
// summary strip on top, the overview panel on the left and the selected file's panel on the right.
// The canvas waits for the fonts as well, so no label is drawn in a fallback font. Loading, a
// failure and an empty map each get a sentence that says what it means and what to do next; an
// empty map is asked for again until files arrive.
import { useEffect, useMemo, useRef, type ReactElement, type RefObject } from 'react';
import { fetchGraph, type GraphSnapshot } from './api/graph';
import { fetchSummary } from './api/summary';
import { Button } from './components/ui/button';
import { TooltipProvider } from './components/ui/tooltip';
import { SearchBox } from './controls/SearchBox';
import { isKeyField, shortcutFor } from './controls/shortcuts';
import { useFontsSettled } from './fonts/fonts';
import { Intro, useIntroOnFirstOpen } from './intro/Intro';
import { MapView } from './map/MapView';
import { ProgramWarmup } from './map/ProgramWarmup';
import { buildScene } from './map/scene';
import { useChromeInsets } from './map/useChromeInsets';
import { FilePanel } from './panels/FilePanel';
import { OverviewPanel } from './panels/OverviewPanel';
import { SummaryStrip } from './panels/SummaryStrip';
import { useMapStore, type LoadState } from './state/store';
import { useIsNarrow } from './theme/media';
import { EMPTY_RECHECK_MS } from './theme/tokens';

/** What a load state that is not ready means, and what the person can do about it. */
export function describeLoad(load: Exclude<LoadState, { status: 'ready' }>): {
  readonly title: string;
  readonly next: string;
} {
  switch (load.status) {
    case 'loading':
      return {
        title: 'Loading the map.',
        next: 'Cortex is sending what it knows about this repository.',
      };
    case 'error':
      return {
        title: `The map could not be loaded (${load.error.code}): ${load.error.message}.`,
        next: 'Check that Cortex is still running (run cortex status in a terminal), then reload this page.',
      };
    case 'empty':
      return {
        title: 'No files in the map yet.',
        next: `Cortex may still be reading this repository. This page asks again every ${EMPTY_RECHECK_MS / 1000} seconds and shows the map as soon as there are files.`,
      };
  }
}

/** Focuses the search box when `/` is pressed outside a text field. */
function useSlashToSearch(input: RefObject<HTMLInputElement | null>): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      const { key, ctrlKey, altKey, metaKey } = event;
      if (
        shortcutFor({ key, ctrlKey, altKey, metaKey, isInField: isKeyField(event.target) }) !==
        'focusSearch'
      )
        return;
      event.preventDefault();
      input.current?.focus();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, [input]);
}

/**
 * Asks for the snapshot again every `EMPTY_RECHECK_MS` while the map is empty, and for the summary
 * once files have arrived, so the summary counts the files the map shows and not the empty index.
 */
function useRecheckWhileEmpty(status: LoadState['status']): void {
  const loadGraph = useMapStore((state) => state.loadGraph);
  const loadSummary = useMapStore((state) => state.loadSummary);
  const wasEmpty = useRef(false);
  useEffect(() => {
    if (status === 'ready' && wasEmpty.current) void loadSummary(fetchSummary);
    wasEmpty.current = status === 'empty';
    if (status !== 'empty') return undefined;
    const timer = window.setInterval(() => {
      void loadGraph(fetchGraph, { isQuiet: true });
    }, EMPTY_RECHECK_MS);
    return () => {
      window.clearInterval(timer);
    };
  }, [status, loadGraph, loadSummary]);
}

/**
 * Whether focus can go back to `element`: still in the page and shown. A search result clicked
 * with the mouse is hidden once the results close, and focusing it would drop focus on the body.
 */
function canTakeFocusBack(element: Element | null): element is HTMLElement {
  return (
    element instanceof HTMLElement &&
    element.isConnected &&
    element !== document.body &&
    element.getClientRects().length > 0
  );
}

/**
 * The button that opens the overview on a narrow screen, where it is a bottom sheet; one sheet
 * shows at a time, so a selected file's panel closes for it.
 */
function OverviewButton(): ReactElement {
  const setOverviewOpen = useMapStore((state) => state.setOverviewOpen);
  const selectFile = useMapStore((state) => state.selectFile);
  return (
    <Button
      onClick={() => {
        selectFile(null);
        setOverviewOpen(true);
      }}
    >
      Overview
    </Button>
  );
}

function ReadyMap({
  snapshot,
  receivedMs,
}: {
  snapshot: GraphSnapshot;
  receivedMs: number;
}): ReactElement {
  const selected = useMapStore((state) => state.selectedFile);
  const isOverviewOpen = useMapStore((state) => state.isOverviewOpen);
  const isNarrow = useIsNarrow();
  const scene = useMemo(() => buildScene(snapshot), [snapshot]);
  const searchRef = useRef<HTMLInputElement>(null);
  const headerRef = useRef<HTMLElement>(null);
  const overviewRef = useRef<HTMLElement>(null);
  // On a narrow screen one sheet shows at a time: a selected file's panel takes the overview's place.
  const isOverviewShown = !isNarrow || (isOverviewOpen && selected === null);
  const chrome = useMemo(
    () => ({
      header: headerRef,
      overview: overviewRef,
      isNarrow,
      isFilePanelOpen: selected !== null,
      isOverviewSheetOpen: isNarrow && isOverviewShown,
    }),
    [isNarrow, selected, isOverviewShown],
  );
  const insets = useChromeInsets(chrome);
  useSlashToSearch(searchRef);
  useIntroOnFirstOpen();
  const restoreFocus = (returnTo: Element | null): void => {
    const target = canTakeFocusBack(returnTo) ? returnTo : searchRef.current;
    target?.focus();
  };
  return (
    <div
      className="app"
      data-panel={selected !== null}
      data-narrow={isNarrow}
      data-sheet={isNarrow && (selected !== null || isOverviewShown)}
    >
      <h1 className="visually-hidden">Cortex map</h1>
      <MapView scene={scene} receivedMs={receivedMs} insets={insets} />
      <header className="topbar" ref={headerRef}>
        <div className="topbar-row">
          <SearchBox scene={scene} inputRef={searchRef} />
          {isNarrow ? <OverviewButton /> : null}
        </div>
        <SummaryStrip snapshot={snapshot} />
      </header>
      {isOverviewShown ? (
        <OverviewPanel snapshot={snapshot} isSheet={isNarrow} panelRef={overviewRef} />
      ) : null}
      {selected === null ? null : <FilePanel scene={scene} id={selected} onClosed={restoreFocus} />}
      <Intro />
    </div>
  );
}

/** The page shown instead of the map while it loads, when it failed, or while it is empty. */
function PageState({ load }: { load: Exclude<LoadState, { status: 'ready' }> }): ReactElement {
  const { title, next } = describeLoad(load);
  return (
    <main className="page-state">
      <h1>Cortex map</h1>
      <p className="page-state-title" role={load.status === 'error' ? 'alert' : 'status'}>
        {title}
      </p>
      <p className="page-state-next">{next}</p>
    </main>
  );
}

/** Fetches the snapshot and the summary once on mount and shows the map or its state. */
export function App(): ReactElement {
  const load = useMapStore((state) => state.load);
  const loadGraph = useMapStore((state) => state.loadGraph);
  const loadSummary = useMapStore((state) => state.loadSummary);
  const cancelLoad = useMapStore((state) => state.cancelLoad);
  const areFontsSettled = useFontsSettled();

  useEffect(() => {
    void loadGraph(fetchGraph);
    void loadSummary(fetchSummary);
    return cancelLoad;
  }, [loadGraph, loadSummary, cancelLoad]);
  useRecheckWhileEmpty(load.status);

  return (
    <TooltipProvider>
      {load.status === 'ready' && areFontsSettled ? (
        <ReadyMap snapshot={load.snapshot} receivedMs={load.receivedMs} />
      ) : (
        <>
          <PageState load={load.status === 'ready' ? { status: 'loading' } : load} />
          {/* The warm-up draws a label too, so it waits for the fonts like the map does. */}
          {load.status !== 'error' && load.status !== 'empty' && areFontsSettled ? (
            <ProgramWarmup />
          ) : null}
        </>
      )}
    </TooltipProvider>
  );
}
