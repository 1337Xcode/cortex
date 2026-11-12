// The page. For now it loads the graph snapshot and states what it got as plain text: loading,
// an error with its cause, an empty repository, or the counts of a ready snapshot.
import { useEffect, type ReactElement } from 'react';
import { fetchGraph } from './api/graph';
import { useMapStore, type LoadState } from './state/store';

/** Says in one sentence what the load state holds. */
export function describeLoad(load: LoadState): string {
  switch (load.status) {
    case 'loading':
      return 'Loading the map.';
    case 'error':
      return `The map could not be loaded (${load.error.code}): ${load.error.message}`;
    case 'empty':
      return 'This repository has no files in the map yet.';
    case 'ready':
      return `${load.snapshot.files.length} files and ${load.snapshot.links.length} links.`;
  }
}

/** Fetches the snapshot once on mount and shows its state. */
export function App(): ReactElement {
  const load = useMapStore((state) => state.load);
  const loadGraph = useMapStore((state) => state.loadGraph);
  const cancelLoad = useMapStore((state) => state.cancelLoad);

  useEffect(() => {
    void loadGraph(fetchGraph);
    return cancelLoad;
  }, [loadGraph, cancelLoad]);

  return (
    <main>
      <h1>Cortex map</h1>
      <p role={load.status === 'error' ? 'alert' : 'status'}>{describeLoad(load)}</p>
    </main>
  );
}
