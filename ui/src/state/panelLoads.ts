// The store's loads for the panels: the summary, the receipts (asked once, when their section
// opens), the activity and the warnings (asked when their sections open), the daemon's health
// (asked when the page opens and when its section does), the selected file's detail, and what the
// health says of the daemon's first reading of the repository. Each load cancels the one of its kind in flight, and an answer that arrives after a
// newer request, or after a cancel, is dropped. A quiet load keeps what is shown until it answers.
import type { Activity } from '../api/activity';
import type { FileDetail } from '../api/file';
import type { ApiError, ApiResult } from '../api/guard';
import type { Health, IndexState } from '../api/health';
import type { Fetcher } from '../api/request';
import type { Receipts } from '../api/receipts';
import type { MapSummary } from '../api/summary';
import type { Warnings } from '../api/warnings';
import type { LoadOptions, RequestSlot, SliceContext } from './sliceContext';
import type { LoadState } from './snapshotSlice';

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

/** What the page knows about the selected file's detail. */
export type DetailState =
  | { readonly status: 'idle' }
  | { readonly status: 'loading'; readonly id: number }
  | { readonly status: 'error'; readonly id: number; readonly error: ApiError }
  | { readonly status: 'ready'; readonly detail: FileDetail };

/** What the page knows about the daemon's first reading of the repository. */
export type IndexKnowledge =
  | { readonly status: 'unknown' }
  | { readonly status: 'error'; readonly error: ApiError }
  | { readonly status: IndexState };

/** Loads one file's detail. */
export type DetailFetcher = (id: number, signal: AbortSignal) => Promise<ApiResult<FileDetail>>;

/** The panels' data with their load states, and the loads that fill them. */
export interface PanelLoads {
  readonly summary: SummaryState;
  readonly receipts: ReceiptsState;
  readonly activity: PanelState<Activity>;
  readonly warnings: PanelState<Warnings>;
  /**
   * The daemon's health as the health section shows it, asked when the page opens; it also says
   * whether Cortex has read the repository's history. The empty map's question sets it too.
   */
  readonly health: PanelState<Health>;
  readonly detail: DetailState;
  /** Whether the daemon is still reading the repository, asked while the map is empty. */
  readonly index: IndexKnowledge;
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
   * Asks the daemon's health, cancelling the question in flight: whether it is still reading the
   * repository, and how it is doing. A quiet question keeps the health shown until it answers.
   */
  readonly loadHealth: (fetcher: Fetcher<Health>, options?: LoadOptions) => Promise<void>;
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

/** The panel state `result` leaves, at `receivedMs`. */
function panelOutcome<T>(result: ApiResult<T>, receivedMs: number): PanelState<T> {
  return result.ok
    ? { status: 'ready', value: result.value, receivedMs }
    : { status: 'error', error: result.error };
}

/** The detail state a detail answer for `id` leaves; a detail of another path is refused. */
function detailOutcome(result: ApiResult<FileDetail>, id: number, load: LoadState): DetailState {
  if (!result.ok) return { status: 'error', id, error: result.error };
  const path = load.status === 'ready' ? load.snapshot.files[id]?.path : undefined;
  if (result.value.path !== path) {
    const message = 'the detail names another path than the map';
    return { status: 'error', id, error: { code: 'bad_range', message } };
  }
  return { status: 'ready', detail: result.value };
}

/** The panels' part of a store built with `context`. */
export function panelLoads({ set, get, requests, clockMs }: SliceContext): PanelLoads {
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

  return {
    summary: { status: 'loading' },
    receipts: { status: 'idle' },
    activity: { status: 'idle' },
    warnings: { status: 'idle' },
    health: { status: 'idle' },
    detail: { status: 'idle' },
    index: { status: 'unknown' },

    loadSummary: async (fetcher, options) => {
      const request = requests.summary.begin();
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
      loadPanel(requests.activity, fetcher, options, (activity) => {
        set({ activity });
      }),

    loadWarnings: (fetcher, options) =>
      loadPanel(requests.warnings, fetcher, options, (warnings) => {
        set({ warnings });
      }),

    loadReceipts: async (fetcher) => {
      const request = requests.receipts.begin();
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
        requests.detail.cancel();
        set({ detail: { status: 'idle' } });
        return;
      }
      const request = requests.detail.begin();
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

    loadHealth: async (fetcher, options) => {
      const request = requests.health.begin();
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
  };
}
