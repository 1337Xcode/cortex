// The page's state: the graph snapshot with its load state, and the selected file. A response
// that arrives after a newer request, or after the page was left, is dropped.
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { GraphError, GraphFetcher, GraphSnapshot } from '../api/graph';

/** What the page knows about the snapshot. */
export type LoadState =
  | { readonly status: 'loading' }
  | { readonly status: 'error'; readonly error: GraphError }
  | { readonly status: 'empty' }
  | { readonly status: 'ready'; readonly snapshot: GraphSnapshot };

/** The store's state and actions. */
export interface MapState {
  readonly load: LoadState;
  /** The id of the selected file, or null; always a valid id of the ready snapshot. */
  readonly selectedFile: number | null;
  /** Counts requests so a late response can tell it was replaced. */
  readonly requestCount: number;
  /** Starts a load, cancelling the one in flight. */
  readonly loadGraph: (fetcher: GraphFetcher) => Promise<void>;
  /** Cancels the load in flight and ignores its response. */
  readonly cancelLoad: () => void;
  /** Selects a file by id; an id that is not in the snapshot clears the selection. */
  readonly selectFile: (id: number | null) => void;
}

/** Creates a store; tests make their own, the page uses `mapStore`. */
export function createMapStore() {
  let controller: AbortController | null = null;

  return createStore<MapState>()((set, get) => ({
    load: { status: 'loading' },
    selectedFile: null,
    requestCount: 0,

    loadGraph: async (fetcher) => {
      controller?.abort();
      const own = new AbortController();
      controller = own;
      const requestCount = get().requestCount + 1;
      set({ load: { status: 'loading' }, selectedFile: null, requestCount });

      const result = await fetcher(own.signal);
      if (get().requestCount !== requestCount) return;
      if (!result.ok) {
        set({ load: { status: 'error', error: result.error } });
      } else if (result.snapshot.files.length === 0) {
        set({ load: { status: 'empty' } });
      } else {
        set({ load: { status: 'ready', snapshot: result.snapshot } });
      }
    },

    cancelLoad: () => {
      controller?.abort();
      controller = null;
      set({ requestCount: get().requestCount + 1 });
    },

    selectFile: (id) => {
      const { load } = get();
      const isKnown =
        id !== null && load.status === 'ready' && id >= 0 && id < load.snapshot.files.length;
      set({ selectedFile: isKnown ? id : null });
    },
  }));
}

/** The page's store. */
export const mapStore = createMapStore();

/** Reads a slice of the page's store from a component. */
export function useMapStore<T>(selector: (state: MapState) => T): T {
  return useStore(mapStore, selector);
}
