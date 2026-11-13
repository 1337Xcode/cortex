// The quiet refresh: while agents work, the numbers the map shows (a file's reads, its tokens, the
// summary, the activity) move on, and the live markers would run ahead of a snapshot read once. So
// each live frame that brings a session's new position, and each answer that finds the snapshot
// older than the daemon's ids, asks for one quiet refresh: of the snapshot, the summary, the
// selected file's detail, and the activity, warnings and health once their sections have loaded
// them; the receipts are read once, when their section first opens. It waits a moment for a
// burst of asks to settle, comes at most once every few seconds, and runs by itself only while the
// map is small enough to read again quickly, or empty. A larger map stamps when its numbers were
// read and is refreshed when the person asks.
import type { StoreApi } from 'zustand/vanilla';
import { REFRESH } from '../theme/tokens';
import type { MapState, Refreshers } from './store';

/** The clock and timers the refresh runs on: the window's, or a test's. */
export interface RefreshTimers {
  readonly nowMs: () => number;
  readonly setTimer: (run: () => void, delayMs: number) => number;
  readonly clearTimer: (handle: number) => void;
}

/** The window's clock and timers. */
export const WINDOW_TIMERS: RefreshTimers = {
  nowMs: () => Date.now(),
  setTimer: (run, delayMs) => window.setTimeout(run, delayMs),
  clearTimer: (handle) => {
    window.clearTimeout(handle);
  },
};

/**
 * Whether the map of `state` refreshes by itself: it is ready and small enough, or empty, so the
 * files the daemon lists after its first reading appear without a reload.
 */
export function refreshesByItself(state: Pick<MapState, 'load'>): boolean {
  const { load } = state;
  if (load.status === 'empty') return true;
  return load.status === 'ready' && load.snapshot.files.length <= REFRESH.autoMaxFiles;
}

/**
 * When the refresh asked for at `nowMs` runs, the last having run at `lastMs` (null for none):
 * `settleMs` from now, and not before `minGapMs` after the last.
 */
export function refreshAt(nowMs: number, lastMs: number | null): number {
  const settled = nowMs + REFRESH.settleMs;
  return lastMs === null ? settled : Math.max(settled, lastMs + REFRESH.minGapMs);
}

/**
 * Refreshes `store` quietly with `refreshers` each time its `refreshSerial` grows, as `refreshAt`
 * spaces them; a new position while one is waiting joins it. Returns the function that stops it.
 */
export function startLiveRefresh(
  store: StoreApi<MapState>,
  refreshers: Refreshers,
  timers: RefreshTimers,
): () => void {
  let waiting: number | null = null;
  let lastMs: number | null = null;
  const run = (): void => {
    waiting = null;
    lastMs = timers.nowMs();
    void store.getState().refresh(refreshers);
  };
  const unsubscribe = store.subscribe((state, previous) => {
    if (state.refreshSerial === previous.refreshSerial || waiting !== null) return;
    if (!refreshesByItself(state)) return;
    waiting = timers.setTimer(run, refreshAt(timers.nowMs(), lastMs) - timers.nowMs());
  });
  return () => {
    unsubscribe();
    if (waiting !== null) timers.clearTimer(waiting);
  };
}
