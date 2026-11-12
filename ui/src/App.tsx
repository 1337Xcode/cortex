// The page: loads the snapshot and the summary, then shows the map with the search box and the
// summary strip on top and the selected file's panel on the right. Loading, error and empty
// repositories each get a plain sentence instead of the map.
import { useEffect, useMemo, useRef, type ReactElement, type RefObject } from 'react';
import { fetchGraph, type GraphSnapshot } from './api/graph';
import { fetchSummary } from './api/summary';
import { SearchBox } from './controls/SearchBox';
import { isKeyField, shortcutFor } from './controls/shortcuts';
import { MapView } from './map/MapView';
import { ProgramWarmup } from './map/ProgramWarmup';
import { buildScene } from './map/scene';
import { useChromeInsets } from './map/useChromeInsets';
import { FilePanel } from './panels/FilePanel';
import { SummaryStrip } from './panels/SummaryStrip';
import { useMapStore, type LoadState } from './state/store';
import { useIsNarrow } from './theme/media';

/** Says in one sentence what a load state that is not ready holds. */
export function describeLoad(load: Exclude<LoadState, { status: 'ready' }>): string {
  switch (load.status) {
    case 'loading':
      return 'Loading the map.';
    case 'error':
      return `The map could not be loaded (${load.error.code}): ${load.error.message}`;
    case 'empty':
      return 'This repository has no files in the map yet.';
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

function ReadyMap({
  snapshot,
  receivedMs,
}: {
  snapshot: GraphSnapshot;
  receivedMs: number;
}): ReactElement {
  const selected = useMapStore((state) => state.selectedFile);
  const isNarrow = useIsNarrow();
  const scene = useMemo(() => buildScene(snapshot), [snapshot]);
  const searchRef = useRef<HTMLInputElement>(null);
  const headerRef = useRef<HTMLElement>(null);
  const insets = useChromeInsets(headerRef, selected !== null, isNarrow);
  useSlashToSearch(searchRef);
  const restoreFocus = (returnTo: Element | null): void => {
    const target = canTakeFocusBack(returnTo) ? returnTo : searchRef.current;
    target?.focus();
  };
  return (
    <div className="app" data-panel={selected !== null} data-narrow={isNarrow}>
      <h1 className="visually-hidden">Cortex map</h1>
      <MapView scene={scene} receivedMs={receivedMs} insets={insets} />
      <header className="topbar" ref={headerRef}>
        <SearchBox scene={scene} inputRef={searchRef} />
        <SummaryStrip snapshot={snapshot} />
      </header>
      {selected === null ? null : <FilePanel scene={scene} id={selected} onClosed={restoreFocus} />}
    </div>
  );
}

/** Fetches the snapshot and the summary once on mount and shows the map or its state. */
export function App(): ReactElement {
  const load = useMapStore((state) => state.load);
  const loadGraph = useMapStore((state) => state.loadGraph);
  const loadSummary = useMapStore((state) => state.loadSummary);
  const cancelLoad = useMapStore((state) => state.cancelLoad);

  useEffect(() => {
    void loadGraph(fetchGraph);
    void loadSummary(fetchSummary);
    return cancelLoad;
  }, [loadGraph, loadSummary, cancelLoad]);

  if (load.status === 'ready')
    return <ReadyMap snapshot={load.snapshot} receivedMs={load.receivedMs} />;
  return (
    <main className="page-state">
      <h1>Cortex map</h1>
      <p role={load.status === 'error' ? 'alert' : 'status'}>{describeLoad(load)}</p>
      {load.status === 'loading' ? <ProgramWarmup /> : null}
    </main>
  );
}
