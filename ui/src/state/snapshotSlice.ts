// The store's snapshot and selection: the graph snapshot and its load state, the selected and
// hovered files (always ids the snapshot holds), the camera's last fly request, the strength below
// which learned links are hidden, the view the map is coloured by, and whether the overview sheet
// and the first-open sentence show. Loading a snapshot clears selection and hover; a response that
// arrives after a newer request, or after a cancel, is dropped.
import type { GraphSnapshot } from '../api/graph';
import type { ApiError } from '../api/guard';
import type { Fetcher } from '../api/request';
import { DEFAULT_MIN_STRENGTH } from '../theme/tokens';
import { DEFAULT_VIEW, VIEWS, type ViewName } from '../views/registry';
import { cancelEvery, type LoadOptions, type SliceContext } from './sliceContext';

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

/**
 * A request to move the camera: to a file, close enough that its folder is open, or to a whole
 * folder, by its path ('' for the repository), framed open. `serial` grows with each request, so
 * asking twice for the same place flies twice.
 */
export type FlyRequest =
  | { readonly target: 'file'; readonly file: number; readonly serial: number }
  | { readonly target: 'folder'; readonly folder: string; readonly serial: number };

/** The snapshot, the selection and the map's settings, with their actions. */
export interface SnapshotSlice {
  readonly load: LoadState;
  /** The id of the selected file, or null; always a valid id of the ready snapshot. */
  readonly selectedFile: number | null;
  /** The last request to move the camera (from search, the lists, the breadcrumb). */
  readonly flyTo: FlyRequest | null;
  /** The id of the file under the pointer, or null; always a valid id of the ready snapshot. */
  readonly hoveredFile: number | null;
  /** Learned links weaker than this (0 to 1) are not drawn. */
  readonly minStrength: number;
  /** The view the map is coloured by. */
  readonly view: ViewName;
  /** Whether the overview panel is open where it can be closed (a narrow screen). */
  readonly isOverviewOpen: boolean;
  /** Whether the sentence that says what the picture is shows. */
  readonly isIntroOpen: boolean;
  /**
   * Starts a snapshot load, cancelling the one in flight; clears selection and hover. A quiet load
   * (asking again while the map is empty) keeps the current state until the answer arrives.
   */
  readonly loadGraph: (fetcher: Fetcher<GraphSnapshot>, options?: LoadOptions) => Promise<void>;
  /** Cancels every load in flight and ignores their responses. */
  readonly cancelLoad: () => void;
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
  readonly setOverviewOpen: (isOpen: boolean) => void;
  readonly setIntroOpen: (isOpen: boolean) => void;
  /** Marks the file under the pointer; an unknown id clears it. */
  readonly hoverFile: (id: number | null) => void;
  /** Sets the link strength threshold, clamped to 0 to 1; a value that is not a number is ignored. */
  readonly setMinStrength: (value: number) => void;
  /** Colours the map by `name`'s view; a name the registry does not have is ignored. */
  readonly setView: (name: string) => void;
}

/** `id` when it is a file of the ready snapshot in `load`, else null. */
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

/** The snapshot and selection part of a store built with `context`. */
export function snapshotSlice({ set, get, requests, clockMs }: SliceContext): SnapshotSlice {
  return {
    load: { status: 'loading' },
    selectedFile: null,
    flyTo: null,
    hoveredFile: null,
    minStrength: DEFAULT_MIN_STRENGTH,
    view: DEFAULT_VIEW,
    isOverviewOpen: false,
    isIntroOpen: false,

    loadGraph: async (fetcher, options) => {
      const request = requests.graph.begin();
      requests.detail.cancel();
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
      cancelEvery(requests);
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

    setView: (name) => {
      const view = VIEWS.find((entry) => entry.name === name);
      if (view !== undefined && view.name !== get().view) set({ view: view.name });
    },
  };
}
