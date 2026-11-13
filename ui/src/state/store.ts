// The page's state, built from three parts that each own one concern: the snapshot and selection
// (`snapshotSlice.ts`: the graph, the selected and hovered files, the camera's fly requests, the
// strength threshold and the view), the panels' loads (`panelLoads.ts`: the summary, the receipts,
// the activity, the warnings, the daemon's health and the selected file's detail), and the live
// sessions with the quiet refresh (`liveSlice.ts`). The parts share one request slot per kind of
// request (`sliceContext.ts`), so a response that arrives after a newer request, or after the
// request was cancelled, is dropped. This module is the store's one entry point: it builds the
// store and names every type the rest of the page reads from it.
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import { liveSlice, type LiveSlice } from './liveSlice';
import { panelLoads, type PanelLoads } from './panelLoads';
import { storeRequests } from './sliceContext';
import { snapshotSlice, type SnapshotSlice } from './snapshotSlice';

export type { LiveStatus, Refreshers } from './liveSlice';
export { positionsKey } from './liveSlice';
export type {
  DetailFetcher,
  DetailState,
  IndexKnowledge,
  PanelState,
  ReceiptsState,
  SummaryState,
} from './panelLoads';
export type { LoadOptions } from './sliceContext';
export type { FlyRequest, LoadState } from './snapshotSlice';

/** The store's state and actions: the three parts together. */
export type MapState = SnapshotSlice & PanelLoads & LiveSlice;

/** Creates a store; tests make their own with a fixed clock, the page uses `mapStore`. */
export function createMapStore(clockMs: () => number = Date.now) {
  const requests = storeRequests();
  return createStore<MapState>()((set, get) => {
    const context = { set, get, requests, clockMs };
    return { ...snapshotSlice(context), ...panelLoads(context), ...liveSlice(context) };
  });
}

/** The page's store. */
export const mapStore = createMapStore();

/** Reads a slice of the page's store from a component. */
export function useMapStore<T>(selector: (state: MapState) => T): T {
  return useStore(mapStore, selector);
}
