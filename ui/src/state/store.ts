// The page's state: the graph snapshot, the summary, the receipts, the activity, the warnings, the
// daemon's health and the selected file's detail with their load states; whether the daemon is
// still reading the repository; the live sessions; the selected and hovered files; the strength
// below which learned links are hidden; and the view the map is coloured by. A response that
// arrives after a newer request, or after the request was cancelled, is dropped. A live frame
// replaces the live state and touches nothing else, except that a frame bringing a session's new
// position counts one more `refreshSerial`, which asks for a quiet refresh (see `liveRefresh.ts`);
// an answer that names a file by an id the snapshot on screen holds another file at counts one too.
import { useStore } from 'zustand';
import { createStore } from 'zustand/vanilla';
import type { Activity } from '../api/activity';
import type { FileDetail } from '../api/file';
import { fileIdOf, type GraphSnapshot } from '../api/graph';
import type { ApiError, ApiResult } from '../api/guard';
import type { Health, IndexState } from '../api/health';
import type { LiveState } from '../api/live';
import type { Fetcher } from '../api/request';
import type { Receipts } from '../api/receipts';
import type { MapSummary } from '../api/summary';
import type { Warnings } from '../api/warnings';
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

/**
 * What the page knows about a section's data that is asked for only when its section opens (the
 * activity, the warnings, the daemon's health): nothing at first, then loading, failed, or ready
 * with the value and when it arrived, by this computer's clock.
 */
export type PanelState<T> =
  | { readonly status: 'idle' }
  | { readonly status: 'loading' }
  | { readonly status: 'error'; readonly error: ApiError }
  | { readonly status: 'ready'; readonly value: T; readonly receivedMs: number };

/** Whether a load shows its loading state (a first load) or keeps what is shown until it answers. */
export interface LoadOptions {
  readonly isQuiet: boolean;
}

/** The loaders a quiet refresh asks again: every one the page has. */
export interface Refreshers {
  readonly graph: Fetcher<GraphSnapshot>;
  readonly summary: Fetcher<MapSummary>;
  readonly detail: DetailFetcher;
  readonly activity: Fetcher<Activity>;
  readonly warnings: Fetcher<Warnings>;
  readonly health: Fetcher<Health>;
}

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
  readonly activity: PanelState<Activity>;
  readonly warnings: PanelState<Warnings>;
  /** The daemon's health as the health section shows it; the empty map's question sets it too. */
  readonly health: PanelState<Health>;
  readonly detail: DetailState;
  /**
   * How many times since the page opened something asked for a quiet refresh: a live frame whose
   * sessions' newest positions differ from the last frame's, or the first answer that found the
   * snapshot on screen out of step with the daemon's ids (see `noteOutOfStep`).
   */
  readonly refreshSerial: number;
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
  readonly loadGraph: (fetcher: Fetcher<GraphSnapshot>, options?: LoadOptions) => Promise<void>;
  /** Cancels the snapshot load in flight and ignores its response. */
  readonly cancelLoad: () => void;
  /** Starts a summary load, cancelling the one in flight; a quiet one keeps the summary shown. */
  readonly loadSummary: (fetcher: Fetcher<MapSummary>, options?: LoadOptions) => Promise<void>;
  /** Starts a receipts load, cancelling the one in flight. */
  readonly loadReceipts: (fetcher: Fetcher<Receipts>) => Promise<void>;
  /** Starts an activity load, cancelling the one in flight; a quiet one keeps the list shown. */
  readonly loadActivity: (fetcher: Fetcher<Activity>, options?: LoadOptions) => Promise<void>;
  /** Starts a warnings load, cancelling the one in flight; a quiet one keeps the list shown. */
  readonly loadWarnings: (fetcher: Fetcher<Warnings>, options?: LoadOptions) => Promise<void>;
  /**
   * Loads the selected file's detail; does nothing when no file is selected. A quiet load keeps
   * the detail shown until the new one arrives.
   */
  readonly loadDetail: (fetcher: DetailFetcher, options?: LoadOptions) => Promise<void>;
  /**
   * Asks every loader again without a loading state: the snapshot (keeping the selected and
   * hovered files, found again by their paths, and the selected file's detail shown while it is
   * read again), the summary, the selected file's detail, and the activity, warnings and health
   * when their sections have loaded them. The receipts are not read again.
   */
  readonly refresh: (refreshers: Refreshers) => Promise<void>;
  /**
   * Says an answer named a file by an id the snapshot on screen holds another file at (or none):
   * the snapshot is older than the daemon's ids. Asks for one quiet refresh per snapshot.
   */
  readonly noteOutOfStep: () => void;
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
  /**
   * Asks the daemon's health, cancelling the question in flight: whether it is still reading the
   * repository, and how it is doing. A quiet question keeps the health shown until it answers.
   */
  readonly loadHealth: (fetcher: Fetcher<Health>, options?: LoadOptions) => Promise<void>;
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

/**
 * `id` of the snapshot `before` as the same file of `after`: its id there, found by its path (ids
 * move when files are added or removed), or null when `after` no longer holds it.
 */
function sameFile(before: LoadState, after: GraphSnapshot, id: number | null): number | null {
  if (id === null || before.status !== 'ready') return null;
  const path = before.snapshot.files[id]?.path;
  return path === undefined ? null : fileIdOf(after.files, path);
}

/**
 * The detail state of file `from`, kept for the same file at its new id `to`: the detail names no
 * other file by id, so only its own id moves. Another file's state, or none, is left as it is.
 */
function movedDetail(detail: DetailState, from: number, to: number): DetailState {
  switch (detail.status) {
    case 'idle':
      return detail;
    case 'ready':
      return detail.detail.id === from
        ? { status: 'ready', detail: { ...detail.detail, id: to } }
        : detail;
    case 'loading':
    case 'error':
      return detail.id === from ? { ...detail, id: to } : detail;
  }
}

/** The failures that say the daemon gave no answer, as against an answer the page could not read. */
const UNANSWERED: ReadonlySet<ApiError['code']> = new Set(['network', 'http_status', 'not_found']);

/**
 * What a failed health question leaves the index as. No answer says Cortex did not say; an answer
 * the page could not read leaves a state already known as it was (the health section shows the
 * error), and an unknown one becomes the error.
 */
function indexAfterFailure(index: IndexKnowledge, error: ApiError): IndexKnowledge {
  const isKnown = index.status === 'reading' || index.status === 'ready';
  return isKnown && !UNANSWERED.has(error.code) ? index : { status: 'error', error };
}

/**
 * Each session's newest position in `state` as one key ("id file at", sessions in their order): two
 * frames with the same key brought no new position.
 */
export function positionsKey(state: LiveState): string {
  return state.sessions
    .map((session) => {
      const newest = session.files[0];
      return newest === undefined ? '' : `${session.id} ${newest.file} ${newest.last_ms}`;
    })
    .filter((key) => key !== '')
    .join('\n');
}

/** The panel state `result` leaves, at `receivedMs`. */
function panelOutcome<T>(result: ApiResult<T>, receivedMs: number): PanelState<T> {
  return result.ok
    ? { status: 'ready', value: result.value, receivedMs }
    : { status: 'error', error: result.error };
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
  const activityRequests = requestSlot();
  const warningsRequests = requestSlot();
  // The last frame's positions, kept through a pause: null until the first frame.
  let lastPositions: string | null = null;
  // The snapshot an answer last found out of step, so each snapshot asks for one refresh only.
  let outOfStepIn: GraphSnapshot | null = null;

  /**
   * Loads one section's data with `fetcher` and hands each state to `put`: loading first unless
   * quiet, then the answer, unless a newer load or a cancel made it stale.
   */
  async function loadPanel<T>(
    slot: RequestSlot,
    fetcher: Fetcher<T>,
    options: LoadOptions | undefined,
    put: (state: PanelState<T>) => void,
  ): Promise<void> {
    const request = slot.begin();
    if (options?.isQuiet !== true) put({ status: 'loading' });
    const result = await fetcher(request.signal);
    if (!request.isCurrent()) return;
    put(panelOutcome(result, clockMs()));
  }

  return createStore<MapState>()((set, get) => ({
    load: { status: 'loading' },
    summary: { status: 'loading' },
    receipts: { status: 'idle' },
    activity: { status: 'idle' },
    warnings: { status: 'idle' },
    health: { status: 'idle' },
    detail: { status: 'idle' },
    refreshSerial: 0,
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
      activityRequests.cancel();
      warningsRequests.cancel();
    },

    loadSummary: async (fetcher, options) => {
      const request = summaryRequests.begin();
      if (options?.isQuiet !== true) set({ summary: { status: 'loading' } });
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      set({
        summary: result.ok
          ? { status: 'ready', summary: result.value }
          : { status: 'error', error: result.error },
      });
    },

    loadActivity: (fetcher, options) =>
      loadPanel(activityRequests, fetcher, options, (activity) => {
        set({ activity });
      }),

    loadWarnings: (fetcher, options) =>
      loadPanel(warningsRequests, fetcher, options, (warnings) => {
        set({ warnings });
      }),

    refresh: async (refreshers) => {
      const before = get().load;
      if (before.status !== 'ready') return;
      const request = graphRequests.begin();
      const loads: Promise<void>[] = [get().loadSummary(refreshers.summary, { isQuiet: true })];
      if (get().activity.status !== 'idle') {
        loads.push(get().loadActivity(refreshers.activity, { isQuiet: true }));
      }
      if (get().warnings.status !== 'idle') {
        loads.push(get().loadWarnings(refreshers.warnings, { isQuiet: true }));
      }
      if (get().health.status !== 'idle') {
        loads.push(get().loadHealth(refreshers.health, { isQuiet: true }));
      }
      const result = await refreshers.graph(request.signal);
      if (request.isCurrent() && result.ok && result.value.files.length > 0) {
        const { selectedFile, hoveredFile, detail } = get();
        const selected = sameFile(before, result.value, selectedFile);
        set({
          load: { status: 'ready', snapshot: result.value, receivedMs: clockMs() },
          selectedFile: selected,
          hoveredFile: sameFile(before, result.value, hoveredFile),
          detail:
            selected === null || selectedFile === null
              ? { status: 'idle' }
              : movedDetail(detail, selectedFile, selected),
        });
        if (selected !== null) loads.push(get().loadDetail(refreshers.detail, { isQuiet: true }));
      }
      // A refresh that failed keeps the snapshot shown: the map stays as it was, its time stamped.
      await Promise.all(loads);
    },

    noteOutOfStep: () => {
      const { load } = get();
      if (load.status !== 'ready' || outOfStepIn === load.snapshot) return;
      outOfStepIn = load.snapshot;
      set({ refreshSerial: get().refreshSerial + 1 });
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

    loadDetail: async (fetcher, options) => {
      const id = get().selectedFile;
      if (id === null) {
        detailRequests.cancel();
        set({ detail: { status: 'idle' } });
        return;
      }
      const request = detailRequests.begin();
      const shown = get().detail;
      const isShown = shown.status === 'ready' && shown.detail.id === id;
      if (options?.isQuiet !== true || !isShown) set({ detail: { status: 'loading', id } });
      const result = await fetcher(id, request.signal);
      if (!request.isCurrent() || get().selectedFile !== id) return;
      const outcome = detailOutcome(result, id, get().load);
      set({ detail: outcome });
      // The daemon holds another file at this id than the snapshot on screen: its ids moved on.
      if (result.ok && outcome.status === 'error') get().noteOutOfStep();
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

    loadHealth: async (fetcher, options) => {
      const request = healthRequests.begin();
      if (options?.isQuiet !== true && get().health.status !== 'ready') {
        set({ health: { status: 'loading' } });
      }
      const result = await fetcher(request.signal);
      if (!request.isCurrent()) return;
      set({
        index: result.ok
          ? { status: result.value.index }
          : indexAfterFailure(get().index, result.error),
        health: panelOutcome(result, clockMs()),
      });
    },

    takeLive: (state) => {
      const positions = positionsKey(state);
      const isNew = lastPositions !== null && positions !== lastPositions;
      lastPositions = positions;
      set({
        live: { status: 'open', state },
        ...(isNew ? { refreshSerial: get().refreshSerial + 1 } : {}),
      });
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
