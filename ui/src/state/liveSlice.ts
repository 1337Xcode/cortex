// The store's live part: the live sessions as the last frame said, and the quiet refresh. A live
// frame replaces the live state and touches nothing else, except that a frame bringing a session's
// new position counts one more `refreshSerial`, which asks for a quiet refresh (see
// `liveRefresh.ts`); an answer or a frame that names a file by an id the snapshot on screen holds
// another file at counts one too, once per snapshot, and so does a frame naming a file while the
// map is empty. The refresh reads every loader again without a loading state, keeping the selected
// and hovered files, found again by their paths; on an empty map it reads the snapshot again too,
// so files the daemon lists after its first reading appear without a reload.
import type { Activity } from '../api/activity';
import { fileIdOf, type GraphSnapshot } from '../api/graph';
import type { Health } from '../api/health';
import type { LiveState } from '../api/live';
import { isFrameAhead } from '../map/liveFiles';
import type { Fetcher } from '../api/request';
import type { MapSummary } from '../api/summary';
import type { Warnings } from '../api/warnings';
import type { DetailFetcher, DetailState } from './panelLoads';
import type { SliceContext } from './sliceContext';
import type { LoadState } from './snapshotSlice';

/**
 * The live view: connecting until the first frame, then open with the last frame's state, or
 * paused while the connection is lost and a new one is on its way.
 */
export type LiveStatus =
  | { readonly status: 'connecting' }
  | { readonly status: 'open'; readonly state: LiveState }
  | { readonly status: 'paused' };

/** The loaders a quiet refresh asks again: every one the page has. */
export interface Refreshers {
  readonly graph: Fetcher<GraphSnapshot>;
  readonly summary: Fetcher<MapSummary>;
  readonly detail: DetailFetcher;
  readonly activity: Fetcher<Activity>;
  readonly warnings: Fetcher<Warnings>;
  readonly health: Fetcher<Health>;
}

/** The live sessions and the quiet refresh, with their actions. */
export interface LiveSlice {
  /** The live sessions, as the last frame said. */
  readonly live: LiveStatus;
  /**
   * How many times since the page opened something asked for a quiet refresh: a live frame whose
   * sessions' newest positions differ from the last frame's, or the first answer that found the
   * snapshot on screen out of step with the daemon's ids (see `noteOutOfStep`).
   */
  readonly refreshSerial: number;
  /**
   * Asks every loader again without a loading state: the snapshot (keeping the selected and
   * hovered files, found again by their paths, and the selected file's detail shown while it is
   * read again), the summary, the selected file's detail, and the activity, warnings and health
   * when they have been loaded. The receipts are not read again. Runs on a ready map and on an
   * empty one, which becomes ready when the snapshot read again holds files.
   */
  readonly refresh: (refreshers: Refreshers) => Promise<void>;
  /**
   * Says an answer named a file by an id the snapshot on screen holds another file at (or none),
   * or named a file while the map is empty: the snapshot is older than the daemon's. Asks for one
   * quiet refresh per snapshot (or per empty answer).
   */
  readonly noteOutOfStep: () => void;
  /** Takes a live frame: the whole live state. */
  readonly takeLive: (state: LiveState) => void;
  /** Marks the live view paused: its connection is lost and another is on its way. */
  readonly pauseLive: () => void;
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

/**
 * Whether the frame `state` says the daemon lists files the snapshot of `load` is out of step
 * with: a file at an id that snapshot holds another file at, or any file while the map is empty.
 */
function isAheadOf(load: LoadState, state: LiveState): boolean {
  if (load.status === 'ready') return isFrameAhead(load.snapshot.files, state);
  return load.status === 'empty' && state.sessions.some((session) => session.files.length > 0);
}

/** The live part of a store built with `context`. */
export function liveSlice({ set, get, requests, clockMs }: SliceContext): LiveSlice {
  // The last frame's positions, kept through a pause: null until the first frame.
  let lastPositions: string | null = null;
  // The load an answer last found out of step, so each snapshot asks for one refresh only.
  let outOfStepIn: LoadState | null = null;

  return {
    live: { status: 'connecting' },
    refreshSerial: 0,

    refresh: async (refreshers) => {
      const before = get().load;
      if (before.status !== 'ready' && before.status !== 'empty') return;
      const request = requests.graph.begin();
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
      if ((load.status !== 'ready' && load.status !== 'empty') || outOfStepIn === load) return;
      outOfStepIn = load;
      set({ refreshSerial: get().refreshSerial + 1 });
    },

    takeLive: (state) => {
      const positions = positionsKey(state);
      const isNew = lastPositions !== null && positions !== lastPositions;
      lastPositions = positions;
      set({
        live: { status: 'open', state },
        ...(isNew ? { refreshSerial: get().refreshSerial + 1 } : {}),
      });
      if (isAheadOf(get().load, state)) get().noteOutOfStep();
    },

    pauseLive: () => {
      set({ live: { status: 'paused' } });
    },
  };
}
