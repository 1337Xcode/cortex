// The page's state: the graph snapshot, the summary, the receipts and the selected file's detail
// with their load states; whether the daemon is still reading the repository; the live sessions; the selected
// and hovered files; the strength below which learned links are hidden; and the view the map is
// coloured by. A response that arrives after a newer request, or after the request was cancelled,
// is dropped. A live frame replaces the live state and touches nothing else.
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { FileDetail } from '../api/file';
import type { GraphSnapshot } from '../api/graph';
import type { ApiError, ApiResult } from '../api/guard';
import type { Health, IndexState } from '../api/health';
import type { LiveState } from '../api/live';
import type { Fetcher } from '../api/request';
import type { Receipts } from '../api/receipts';
import type { MapSummary } from '../api/summary';
import { DEFAULT_MIN_STRENGTH } from '../theme/tokens';
import { DEFAULT_VIEW, VIEWS, type ViewName } from '../views/registry';

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

/** What the page knows about the receipts: nothing until the receipts panel first asks for them. */
export type ReceiptsState =
  | { readonly status: 'idle' }
  | { readonly status: 'loading' }
  | { readonly status: 'error'; readonly error: ApiError }
  | { readonly status: 'ready'; readonly receipts: Receipts };

/** What the page knows about the selected file's detail. */
export type DetailState =
  | { readonly status: 'idle' }
  | { readonly status: 'loading'; readonly id: number }
  | { readonly status: 'error'; readonly id: number; readonly error: ApiError }
  | { readonly status: 'ready'; readonly detail: FileDetail };

/**
 * A request to move the camera: to a file, close enough that its folder is open, or to a whole
 * folder, by its path ('' for the repository), framed open. `serial` grows with each request, so
 * asking twice for the same place flies twice.
 */
export type FlyRequest =
  | { readonly target: 'file'; readonly file: number; readonly serial: number }
  | { readonly target: 'folder'; readonly folder: string; readonly serial: number };

/** What the page knows about the daemon's first reading of the repository. */
export type IndexKnowledge =
  | { readonly status: 'unknown' }
  | { readonly status: 'error'; readonly error: ApiError }
  | { readonly status: IndexState };

/**
 * The live view: connecting until the first frame, then open with the last frame's state, or
 * paused while the connection is lost and a new one is on its way.
 */
export type LiveStatus =
  | { readonly status: 'connecting' }
  | { readonly status: 'open'; readonly state: LiveState }
  | { readonly status: 'paused' };

/** Loads one file's detail. */
export type DetailFetcher = (id: number, signal: AbortSignal) => Promise<ApiResult<FileDetail>>;

/** The store's state and actions. */
export interface MapState {
  readonly load: LoadState;
  readonly summary: SummaryState;
  readonly receipts: ReceiptsState;
  readonly detail: DetailState;
  /** The id of the selected file, or null; always a valid id of the ready snapshot. */
  readonly selectedFile: number | null;
  /** The last request to move the camera (from search, the lists, the breadcrumb). */
  readonly flyTo: FlyRequest | null;
  /** The id of the file under the pointer, or null; always a valid id of the ready snapshot. */
  readonly hoveredFile: number | null;
  /** Learned links weaker than this (0 to 1) are not drawn. */
  readonly minStrength: number;
  /** Whether the daemon is still reading the repository, asked while the map is empty. */
  readonly index: IndexKnowledge;
  /** The live sessions, as the last frame said. */
  readonly live: LiveStatus;
  /** The view the map is coloured by. */
  readonly view: ViewName;
  /**
   * Starts a snapshot load, cancelling the one in flight; clears selection and hover. A quiet load
   * (asking again while the map is empty) keeps the current state until the answer arrives.
   */
  readonly loadGraph: (
    fetcher: Fetcher<GraphSnapshot>,
    options?: { readonly isQuiet: boolean },
  ) => Promise<void>;
  /** Cancels the snapshot load in flight and ignores its response. */
  readonly cancelLoad: () => void;
  /** Starts a summary load, cancelling the one in flight. */
  readonly loadSummary: (fetcher: Fetcher<MapSummary>) => Promise<void>;
  /** Starts a receipts load, cancelling the one in flight. */
  readonly loadReceipts: (fetcher: Fetcher<Receipts>) => Promise<void>;
  /** Loads the selected file's detail; does nothing when no file is selected. */
  readonly loadDetail: (fetcher: DetailFetcher) => Promise<void>;
  /**
   * Selects a file by id; an id that is not in the snapshot clears the selection. With `fly`, also
   * asks the camera to move to the file.
   */
  readonly selectFile: (id: number | null, options?: { readonly fly: boolean }) => void;
  /**
   * Asks the camera to frame folder `path` whole and open, keeping the selection; the keyboard's
   * way into a folder drawn collapsed. A path no file of the snapshot sits under does nothing.
   */
  readonly showFolder: (path: string) => void;
  /** Whether the overview panel is open where it can be closed (a narrow screen). */
  readonly isOverviewOpen: boolean;
  readonly setOverviewOpen: (isOpen: boolean) => void;
  /** Whether the sentence that says what the picture is shows. */
  readonly isIntroOpen: boolean;
  readonly setIntroOpen: (isOpen: boolean) => void;
  /** Marks the file under the pointer; an unknown id clears it. */
  readonly hoverFile: (id: number | null) => void;
  /** Sets the link strength threshold, clamped to 0 to 1; a value that is not a number is ignored. */
  readonly setMinStrength: (value: number) => void;
  /** Asks the daemon whether it is still reading the repository, cancelling the question in flight. */
  readonly loadHealth: (fetcher: Fetcher<Health>) => Promise<void>;
  /** Takes a live frame: the whole live state. */
  readonly takeLive: (state: LiveState) => void;
  /** Marks the live view paused: its connection is lost and another is on its way. */
  readonly pauseLive: () => void;
  /** Colours the map by `name`'s view; a name the registry does not have is ignored. */
  readonly setView: (name: string) => void;
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

/** Whether `path` is the repository ('') or a folder some file of the snapshot sits under. */
function isFolderOf(load: LoadState, path: string): boolean {
  if (load.status !== 'ready') return false;
  if (path === '') return true;
  const prefix = `${path}/`;
  return load.snapshot.files.some((file) => file.path.startsWith(prefix));
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
  const receiptsRequests = requestSlot();
  const detailRequests = requestSlot();
  const healthRequests = requestSlot();

  return createStore<MapState>()((set, get) => ({
    load: { status: 'loading' },
    summary: { status: 'loading' },
    receipts: { status: 'idle' },
    detail: { status: 'idle' },
    selectedFile: null,
    flyTo: null,
    hoveredFile: null,
    minStrength: DEFAULT_MIN_STRENGTH,
    index: { status: 'unknown' },
    live: { status: 'connecting' },
    view: DEFAULT_VIEW,
    isOverviewOpen: false,
    isIntroOpen: false,

    loadGraph: async (fetcher, options) => {
      const request = graphRequests.begin();
      detailRequests.cancel();
      const reset = { selectedFile: null, flyTo: null, hoveredFile: null };
      if (options?.isQuiet === true) set(reset);
      else set({ ...reset, load: { status: 'loading' }, detail: { status: 'idle' } });
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      if (!result.ok) set({ load: { status: 'error', error: result.error } });
      else if (result.value.files.length === 0) set({ load: { status: 'empty' } });
      else set({ load: { status: 'ready', snapshot: result.value, receivedMs: clockMs() } });
    },

    cancelLoad: () => {
      graphRequests.cancel();
      summaryRequests.cancel();
      receiptsRequests.cancel();
      detailRequests.cancel();
      healthRequests.cancel();
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

    loadReceipts: async (fetcher) => {
      const request = receiptsRequests.begin();
      set({ receipts: { status: 'loading' } });
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      set({
        receipts: result.ok
          ? { status: 'ready', receipts: result.value }
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
      set({ selectedFile: selected, flyTo: { target: 'file', file: selected, serial } });
    },

    showFolder: (path) => {
      if (!isFolderOf(get().load, path)) return;
      const serial = (get().flyTo?.serial ?? 0) + 1;
      set({ flyTo: { target: 'folder', folder: path, serial } });
    },

    setOverviewOpen: (isOpen) => {
      set({ isOverviewOpen: isOpen });
    },

    setIntroOpen: (isOpen) => {
      set({ isIntroOpen: isOpen });
    },

    hoverFile: (id) => {
      const hovered = knownId(get().load, id);
      if (hovered !== get().hoveredFile) set({ hoveredFile: hovered });
    },

    setMinStrength: (value) => {
      if (!Number.isFinite(value)) return;
      set({ minStrength: Math.min(1, Math.max(0, value)) });
    },

    loadHealth: async (fetcher) => {
      const request = healthRequests.begin();
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      set({
        index: result.ok
          ? { status: result.value.index }
          : { status: 'error', error: result.error },
      });
    },

    takeLive: (state) => {
      set({ live: { status: 'open', state } });
    },

    pauseLive: () => {
      set({ live: { status: 'paused' } });
    },

    setView: (name) => {
      const view = VIEWS.find((entry) => entry.name === name);
      if (view !== undefined && view.name !== get().view) set({ view: view.name });
    },
  }));
}

/** The page's store. */
export const mapStore = createMapStore();

/** Reads a slice of the page's store from a component. */
export function useMapStore<T>(selector: (state: MapState) => T): T {
  return useStore(mapStore, selector);
}
