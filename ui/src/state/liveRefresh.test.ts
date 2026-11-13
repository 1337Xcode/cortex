// Tests for the quiet refresh: a live frame that brings a session's new position counts once, a
// burst of them becomes one quiet snapshot load after the settle time, refreshes keep their gap,
// the selected and hovered files and the open detail survive a refresh that moves their ids, an
// answer or a frame that finds the snapshot out of step asks once, an empty map is read again when
// a frame names a file, and a large map waits to be asked.
import { describe, expect, it } from 'vitest';
import type { FileDetail } from '../api/file';
import type { GraphSnapshot } from '../api/graph';
import type { ApiResult } from '../api/guard';
import type { LiveState } from '../api/live';
import { testFile } from '../map/testScene';
import { REFRESH } from '../theme/tokens';
import { refreshAt, startLiveRefresh, type RefreshTimers } from './liveRefresh';
import { createMapStore, positionsKey, type DetailFetcher, type Refreshers } from './store';

const NOW_MS = 1_762_900_000_000;

function snapshotOf(count: number): GraphSnapshot {
  const files = Array.from({ length: count }, (_, id) =>
    testFile(id, `f${String(id).padStart(6, '0')}.rs`),
  );
  return { version: 1, files, links: [] };
}

/**
 * A frame with one session whose newest position is `file` at `lastMs`, named by `path` (by
 * default the path `snapshotOf` gives that id).
 */
function frame(
  file: number,
  lastMs: number,
  path = `f${String(file).padStart(6, '0')}.rs`,
): LiveState {
  return {
    version: 1,
    sessions: [
      {
        id: 's1',
        agent: 'claude',
        title: 'Claude Code',
        files: [{ file, path, mode: 'read', last_ms: lastMs }],
      },
    ],
  };
}

/** A clock and timers that move only when the test says. */
function fakeTimers(): RefreshTimers & { advance: (ms: number) => void } {
  let now = NOW_MS;
  let next = 1;
  const waiting = new Map<number, { at: number; run: () => void }>();
  return {
    nowMs: () => now,
    setTimer: (run, delayMs) => {
      const handle = next;
      next += 1;
      waiting.set(handle, { at: now + delayMs, run });
      return handle;
    },
    clearTimer: (handle) => {
      waiting.delete(handle);
    },
    advance: (ms) => {
      now += ms;
      for (const [handle, timer] of [...waiting]) {
        if (timer.at > now) continue;
        waiting.delete(handle);
        timer.run();
      }
    },
  };
}

/** Refreshers that count the snapshot loads and answer `snapshot`. */
function counting(snapshot: GraphSnapshot): { refreshers: Refreshers; graphLoads: () => number } {
  let loads = 0;
  const never = (): Promise<ApiResult<never>> =>
    Promise.resolve({ ok: false, error: { code: 'network', message: 'not asked' } });
  return {
    graphLoads: () => loads,
    refreshers: {
      graph: () => {
        loads += 1;
        return Promise.resolve({ ok: true, value: snapshot });
      },
      summary: never,
      detail: never,
      activity: never,
      warnings: never,
      health: never,
    },
  };
}

/** A promise that settles once `store` holds a snapshot other than the one it holds now. */
function snapshotReplaced(store: ReturnType<typeof createMapStore>): Promise<void> {
  const before = store.getState().load;
  return new Promise((resolve) => {
    const unsubscribe = store.subscribe((state) => {
      if (state.load === before) return;
      unsubscribe();
      resolve();
    });
  });
}

/** A file's detail as the daemon answers it, for `id` at `path`. */
function detailOf(id: number, path: string): FileDetail {
  return {
    version: 1,
    id,
    path,
    facts: [],
    history: [],
    history_total: 0,
    history_reader: 'off',
    symbols: [],
    symbols_total: 0,
    selected: [],
    told: [],
    editor_url: null,
  };
}

/** A detail fetcher that answers only when the test says, recording the ids asked. */
function heldDetail(paths: readonly string[]): {
  fetcher: DetailFetcher;
  asked: number[];
  answer: () => void;
} {
  const asked: number[] = [];
  const waiting: (() => void)[] = [];
  const fetcher: DetailFetcher = (id) => {
    asked.push(id);
    return new Promise((resolve) => {
      waiting.push(() => {
        resolve({ ok: true, value: detailOf(id, paths[id] ?? '') });
      });
    });
  };
  return {
    fetcher,
    asked,
    answer: () => {
      for (const settle of waiting.splice(0)) settle();
    },
  };
}

describe('a new position', () => {
  it('counts once per frame that moves a session, never for the first frame or the same one', () => {
    const store = createMapStore(() => NOW_MS);
    store.getState().takeLive(frame(1, 10));
    expect(store.getState().refreshSerial).toBe(0);
    store.getState().takeLive(frame(1, 10));
    expect(store.getState().refreshSerial).toBe(0);
    store.getState().takeLive(frame(2, 20));
    expect(store.getState().refreshSerial).toBe(1);
    expect(positionsKey(frame(2, 20))).toBe('s1 2 20');
  });
});

describe('the quiet refresh', () => {
  it('turns a burst of new positions into one quiet snapshot load after the settle time', async () => {
    const store = createMapStore(() => NOW_MS);
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: snapshotOf(3) }));
    store.getState().selectFile(1);
    const timers = fakeTimers();
    const { refreshers, graphLoads } = counting(snapshotOf(3));
    const stop = startLiveRefresh(store, refreshers, timers);
    store.getState().takeLive(frame(0, 1));
    store.getState().takeLive(frame(1, 2));
    store.getState().takeLive(frame(2, 3));
    timers.advance(REFRESH.settleMs - 1);
    expect(graphLoads()).toBe(0);
    const replaced = snapshotReplaced(store);
    timers.advance(1);
    expect(graphLoads()).toBe(1);
    await replaced;
    // Quiet: the map never went back to loading, and the selected file, still at its id, stays.
    expect(store.getState().load.status).toBe('ready');
    expect(store.getState().selectedFile).toBe(1);
    stop();
  });

  it('keeps its gap: the next refresh comes no sooner than the gap after the last', () => {
    expect(refreshAt(NOW_MS, null)).toBe(NOW_MS + REFRESH.settleMs);
    expect(refreshAt(NOW_MS + 1000, NOW_MS)).toBe(NOW_MS + REFRESH.minGapMs);
    expect(refreshAt(NOW_MS + 60_000, NOW_MS)).toBe(NOW_MS + 60_000 + REFRESH.settleMs);
  });

  it('leaves a map larger than it refreshes by itself to be read again on request', async () => {
    const store = createMapStore(() => NOW_MS);
    const large = snapshotOf(REFRESH.autoMaxFiles + 1);
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: large }));
    const timers = fakeTimers();
    const { refreshers, graphLoads } = counting(large);
    const stop = startLiveRefresh(store, refreshers, timers);
    store.getState().takeLive(frame(0, 1));
    store.getState().takeLive(frame(1, 2));
    timers.advance(REFRESH.minGapMs * 2);
    expect(graphLoads()).toBe(0);
    stop();
  });

  it('keeps the selected and hovered files by path when a file is added before them, the detail open', async () => {
    const store = createMapStore(() => NOW_MS);
    const before: GraphSnapshot = {
      version: 1,
      files: [testFile(0, 'b.rs'), testFile(1, 'c.rs'), testFile(2, 'd.rs')],
      links: [],
    };
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: before }));
    store.getState().selectFile(1);
    store.getState().hoverFile(2);
    await store
      .getState()
      .loadDetail((id) => Promise.resolve({ ok: true, value: detailOf(id, 'c.rs') }));
    // a.rs is added: every id moves up by one.
    const after: GraphSnapshot = {
      version: 1,
      files: [testFile(0, 'a.rs'), testFile(1, 'b.rs'), testFile(2, 'c.rs'), testFile(3, 'd.rs')],
      links: [],
    };
    const held = heldDetail(after.files.map((file) => file.path));
    const refreshers = { ...counting(after).refreshers, detail: held.fetcher };
    const refreshed = store.getState().refresh(refreshers);
    await snapshotReplaced(store);
    expect([store.getState().selectedFile, store.getState().hoveredFile]).toEqual([2, 3]);
    // While the detail is read again, the one shown stays, for the same file at its new id.
    expect(store.getState().detail).toEqual({ status: 'ready', detail: detailOf(2, 'c.rs') });
    expect(held.asked).toEqual([2]);
    held.answer();
    await refreshed;
    expect(store.getState().detail).toEqual({ status: 'ready', detail: detailOf(2, 'c.rs') });
  });

  it('drops the selection and the detail when the new snapshot no longer holds the file', async () => {
    const store = createMapStore(() => NOW_MS);
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: snapshotOf(3) }));
    store.getState().selectFile(2);
    const shorter: GraphSnapshot = {
      version: 1,
      files: [testFile(0, 'f000000.rs'), testFile(1, 'f000001.rs')],
      links: [],
    };
    await store.getState().refresh(counting(shorter).refreshers);
    expect(store.getState().selectedFile).toBeNull();
    expect(store.getState().detail).toEqual({ status: 'idle' });
  });

  it('asks for one quiet refresh when an answer finds the snapshot out of step', async () => {
    const store = createMapStore(() => NOW_MS);
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: snapshotOf(3) }));
    const timers = fakeTimers();
    const { refreshers, graphLoads } = counting(snapshotOf(4));
    const stop = startLiveRefresh(store, refreshers, timers);
    store.getState().noteOutOfStep();
    store.getState().noteOutOfStep();
    expect(store.getState().refreshSerial).toBe(1);
    const replaced = snapshotReplaced(store);
    timers.advance(REFRESH.settleMs);
    expect(graphLoads()).toBe(1);
    await replaced;
    // A new snapshot may be out of step again, and asks again, once.
    store.getState().noteOutOfStep();
    store.getState().noteOutOfStep();
    expect(store.getState().refreshSerial).toBe(2);
    stop();
  });

  it('asks for one quiet refresh when a frame names a file at an id the snapshot holds another at', async () => {
    const store = createMapStore(() => NOW_MS);
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: snapshotOf(3) }));
    // The first frame brings no new position, but its file 1 is f000002.rs, which the snapshot
    // holds at 2: the daemon's ids moved on.
    store.getState().takeLive(frame(1, 10));
    expect(store.getState().refreshSerial).toBe(0);
    store.getState().takeLive(frame(1, 10, 'f000002.rs'));
    expect(store.getState().refreshSerial).toBe(1);
    store.getState().takeLive(frame(1, 10, 'f000002.rs'));
    expect(store.getState().refreshSerial).toBe(1);
  });

  it('reads an empty map again when a frame names a file, and shows the files it then holds', async () => {
    const store = createMapStore(() => NOW_MS);
    const empty: GraphSnapshot = { version: 1, files: [], links: [] };
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: empty }));
    expect(store.getState().load.status).toBe('empty');
    const timers = fakeTimers();
    const { refreshers, graphLoads } = counting(snapshotOf(2));
    const stop = startLiveRefresh(store, refreshers, timers);
    // A first frame names a file: the daemon lists files the empty map does not.
    store.getState().takeLive(frame(0, 1));
    expect(store.getState().refreshSerial).toBe(1);
    const replaced = snapshotReplaced(store);
    timers.advance(REFRESH.settleMs);
    expect(graphLoads()).toBe(1);
    await replaced;
    const { load } = store.getState();
    expect(load.status === 'ready' ? load.snapshot.files.map((file) => file.path) : load).toEqual([
      'f000000.rs',
      'f000001.rs',
    ]);
    stop();
  });

  it('keeps an empty map empty when the snapshot read again is still empty', async () => {
    const store = createMapStore(() => NOW_MS);
    const empty: GraphSnapshot = { version: 1, files: [], links: [] };
    await store.getState().loadGraph(() => Promise.resolve({ ok: true, value: empty }));
    const { refreshers, graphLoads } = counting(empty);
    await store.getState().refresh(refreshers);
    expect(graphLoads()).toBe(1);
    expect(store.getState().load).toEqual({ status: 'empty' });
  });
});
