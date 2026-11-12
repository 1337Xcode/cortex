// The page's state: the graph snapshot, the summary and the selected file's detail with their
// load states; the selected and hovered files; and the strength below which learned links are
// hidden. A response that arrives after a newer request, or after the request was cancelled, is
// dropped.
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { FileDetail } from '../api/file';
import type { GraphSnapshot } from '../api/graph';
import type { ApiError, ApiResult } from '../api/guard';
import type { Fetcher } from '../api/request';
import type { MapSummary } from '../api/summary';
import { DEFAULT_MIN_STRENGTH } from '../theme/tokens';

/** What the page knows about the snapshot. */
export type LoadState =
  | { readonly status: 'loading' }
  | { readonly status: 'error'; readonly error: ApiError }
  | { readonly status: 'empty' }
  | {
      readonly status: 'ready';
      readonly snapshot: GraphSnapshot;
      /** When the snapshot arrived, in ms since the epoch: "the last 24 hours" ends here. */
      readonly receivedMs: number;
    };

/** What the page knows about the summary. */
export type SummaryState =
  | { readonly status: 'loading' }
  | { readonly status: 'error'; readonly error: ApiError }
  | { readonly status: 'ready'; readonly summary: MapSummary };

/** What the page knows about the selected file's detail. */
export type DetailState =
  | { readonly status: 'idle' }
  | { readonly status: 'loading'; readonly id: number }
  | { readonly status: 'error'; readonly id: number; readonly error: ApiError }
  | { readonly status: 'ready'; readonly detail: FileDetail };

/**
 * What a fly request frames: the file itself, close enough that its folder is open, or the whole
 * folder the file is in.
 */
export type FlyTarget = 'file' | 'folder';

/** A request to move the camera to a file or to its folder. */
export interface FlyRequest {
  readonly file: number;
  readonly target: FlyTarget;
  readonly serial: number;
}

/** Loads one file's detail. */
export type DetailFetcher = (id: number, signal: AbortSignal) => Promise<ApiResult<FileDetail>>;

/** The store's state and actions. */
export interface MapState {
  readonly load: LoadState;
  readonly summary: SummaryState;
  readonly detail: DetailState;
  /** The id of the selected file, or null; always a valid id of the ready snapshot. */
  readonly selectedFile: number | null;
  /**
   * The last request to move the camera to a selected file (from search, the summary or the side
   * panel); `serial` grows with each request, so asking twice for the same file flies twice.
   */
  readonly flyTo: FlyRequest | null;
  /** The id of the file under the pointer, or null; always a valid id of the ready snapshot. */
  readonly hoveredFile: number | null;
  /** Learned links weaker than this (0 to 1) are not drawn. */
  readonly minStrength: number;
  /** Starts a snapshot load, cancelling the one in flight; clears selection and hover. */
  readonly loadGraph: (fetcher: Fetcher<GraphSnapshot>) => Promise<void>;
  /** Cancels the snapshot load in flight and ignores its response. */
  readonly cancelLoad: () => void;
  /** Starts a summary load, cancelling the one in flight. */
  readonly loadSummary: (fetcher: Fetcher<MapSummary>) => Promise<void>;
  /** Loads the selected file's detail; does nothing when no file is selected. */
  readonly loadDetail: (fetcher: DetailFetcher) => Promise<void>;
  /**
   * Selects a file by id; an id that is not in the snapshot clears the selection. With `fly`, also
   * asks the camera to move to the file.
   */
  readonly selectFile: (id: number | null, options?: { readonly fly: boolean }) => void;
  /**
   * Keeps file `id` selected and asks the camera to frame the whole folder it is in, open; the
   * keyboard's way into a folder drawn collapsed. An unknown id does nothing.
   */
  readonly showFolderOf: (id: number) => void;
  /** Marks the file under the pointer; an unknown id clears it. */
  readonly hoverFile: (id: number | null) => void;
  /** Sets the link strength threshold, clamped to 0 to 1; a value that is not a number is ignored. */
  readonly setMinStrength: (value: number) => void;
}

/** One kind of request: a newer start or a cancel makes every older start stale. */
interface RequestSlot {
  readonly begin: () => { readonly signal: AbortSignal; readonly isCurrent: () => boolean };
  readonly cancel: () => void;
}

function requestSlot(): RequestSlot {
  let controller: AbortController | null = null;
  let generation = 0;
  const cancel = (): void => {
    controller?.abort();
    controller = null;
    generation += 1;
  };
  return {
    begin: () => {
      cancel();
      const own = new AbortController();
      controller = own;
      const mine = generation;
      return { signal: own.signal, isCurrent: () => generation === mine };
    },
    cancel,
  };
}

function knownId(load: LoadState, id: number | null): number | null {
  if (id === null || load.status !== 'ready') return null;
  return Number.isInteger(id) && id >= 0 && id < load.snapshot.files.length ? id : null;
}

function detailOutcome(result: ApiResult<FileDetail>, id: number, load: LoadState): DetailState {
  if (!result.ok) return { status: 'error', id, error: result.error };
  const path = load.status === 'ready' ? load.snapshot.files[id]?.path : undefined;
  if (result.value.path !== path) {
    const message = 'the detail names another path than the map';
    return { status: 'error', id, error: { code: 'bad_range', message } };
  }
  return { status: 'ready', detail: result.value };
}

/** Creates a store; tests make their own with a fixed clock, the page uses `mapStore`. */
export function createMapStore(clockMs: () => number = Date.now) {
  const graphRequests = requestSlot();
  const summaryRequests = requestSlot();
  const detailRequests = requestSlot();

  return createStore<MapState>()((set, get) => ({
    load: { status: 'loading' },
    summary: { status: 'loading' },
    detail: { status: 'idle' },
    selectedFile: null,
    flyTo: null,
    hoveredFile: null,
    minStrength: DEFAULT_MIN_STRENGTH,

    loadGraph: async (fetcher) => {
      const request = graphRequests.begin();
      detailRequests.cancel();
      set({
        load: { status: 'loading' },
        selectedFile: null,
        flyTo: null,
        hoveredFile: null,
        detail: { status: 'idle' },
      });
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      if (!result.ok) set({ load: { status: 'error', error: result.error } });
      else if (result.value.files.length === 0) set({ load: { status: 'empty' } });
      else set({ load: { status: 'ready', snapshot: result.value, receivedMs: clockMs() } });
    },

    cancelLoad: () => {
      graphRequests.cancel();
      summaryRequests.cancel();
      detailRequests.cancel();
    },

    loadSummary: async (fetcher) => {
      const request = summaryRequests.begin();
      set({ summary: { status: 'loading' } });
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      set({
        summary: result.ok
          ? { status: 'ready', summary: result.value }
          : { status: 'error', error: result.error },
      });
    },

    loadDetail: async (fetcher) => {
      const id = get().selectedFile;
      if (id === null) {
        detailRequests.cancel();
        set({ detail: { status: 'idle' } });
        return;
      }
      const request = detailRequests.begin();
      set({ detail: { status: 'loading', id } });
      const result = await fetcher(id, request.signal);
      if (!request.isCurrent() || get().selectedFile !== id) return;
      set({ detail: detailOutcome(result, id, get().load) });
    },

    selectFile: (id, options) => {
      const selected = knownId(get().load, id);
      if (selected === null || options?.fly !== true) {
        set({ selectedFile: selected });
        return;
      }
      const serial = (get().flyTo?.serial ?? 0) + 1;
      set({ selectedFile: selected, flyTo: { file: selected, target: 'file', serial } });
    },

    showFolderOf: (id) => {
      const selected = knownId(get().load, id);
      if (selected === null) return;
      const serial = (get().flyTo?.serial ?? 0) + 1;
      set({ selectedFile: selected, flyTo: { file: selected, target: 'folder', serial } });
    },

    hoverFile: (id) => {
      const hovered = knownId(get().load, id);
      if (hovered !== get().hoveredFile) set({ hoveredFile: hovered });
    },

    setMinStrength: (value) => {
      if (!Number.isFinite(value)) return;
      set({ minStrength: Math.min(1, Math.max(0, value)) });
    },
  }));
}

/** The page's store. */
export const mapStore = createMapStore();

/** Reads a slice of the page's store from a component. */
export function useMapStore<T>(selector: (state: MapState) => T): T {
  return useStore(mapStore, selector);
}
