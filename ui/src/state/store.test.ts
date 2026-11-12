// Tests for the store: each load state, selection and hover rules, the strength threshold, the
// summary and detail loads, and dropping late responses.
import { describe, expect, it } from 'vitest';
import type { FileDetail } from '../api/file';
import type { GraphSnapshot } from '../api/graph';
import type { ApiResult } from '../api/guard';
import type { MapSummary } from '../api/summary';
import { DEFAULT_MIN_STRENGTH } from '../theme/tokens';
import { createMapStore } from './store';

function snapshotOf(count: number): GraphSnapshot {
  const files = Array.from({ length: count }, (_, id) => ({
    id,
    path: `f${String(id).padStart(3, '0')}.rs`,
    lines: 1,
    interest: 0,
    tokens: 0,
    touched_ms: null,
    facts: 0,
    suspect: 0,
    reads: 0,
    commits: 0,
  }));
  return { version: 1, files, links: [] };
}

function ready(count: number): ApiResult<GraphSnapshot> {
  return { ok: true, value: snapshotOf(count) };
}

function detailFor(id: number, path = `f${String(id).padStart(3, '0')}.rs`): FileDetail {
  return { version: 1, id, path, facts: [], history: [], told: [] };
}

/** The fixed clock of the tests' stores. */
const NOW_MS = 1_762_900_000_000;

const SUMMARY: MapSummary = {
  version: 1,
  files: 3,
  folders: 0,
  explored_files: 0,
  learned: false,
  strongest_pair: null,
  busiest_folder: null,
  window_days: 7,
};

/** A fetcher the test settles by hand, to order responses exactly. */
function deferred<T>(): {
  fetcher: (signal: AbortSignal) => Promise<T>;
  settle: (value: T) => void;
  signal: () => AbortSignal | null;
} {
  let settle: (value: T) => void = () => undefined;
  let seen: AbortSignal | null = null;
  const fetcher = (signal: AbortSignal): Promise<T> => {
    seen = signal;
    return new Promise((resolve) => {
      settle = resolve;
    });
  };
  return {
    fetcher,
    settle: (value) => {
      settle(value);
    },
    signal: () => seen,
  };
}

async function readyStore(count: number): Promise<ReturnType<typeof createMapStore>> {
  const store = createMapStore(() => NOW_MS);
  await store.getState().loadGraph(() => Promise.resolve(ready(count)));
  return store;
}

describe('map store: the snapshot', () => {
  it('starts loading with nothing selected or hovered and the default threshold', () => {
    const state = createMapStore().getState();
    expect(state.load).toEqual({ status: 'loading' });
    expect(state.summary).toEqual({ status: 'loading' });
    expect(state.detail).toEqual({ status: 'idle' });
    expect([state.selectedFile, state.hoveredFile]).toEqual([null, null]);
    expect(state.minStrength).toBe(DEFAULT_MIN_STRENGTH);
  });

  it('becomes ready with the snapshot, or empty when it has no files', async () => {
    expect((await readyStore(3)).getState().load).toEqual({
      status: 'ready',
      snapshot: snapshotOf(3),
      receivedMs: NOW_MS,
    });
    expect((await readyStore(0)).getState().load).toEqual({ status: 'empty' });
  });

  it('keeps the typed error when the load fails', async () => {
    const store = createMapStore();
    const error = { code: 'http_status', message: 'the daemon answered 500' } as const;
    await store.getState().loadGraph(() => Promise.resolve({ ok: false, error }));
    expect(store.getState().load).toEqual({ status: 'error', error });
  });

  it('ignores an older response that arrives after a newer request', async () => {
    const store = createMapStore(() => NOW_MS);
    const first = deferred<ApiResult<GraphSnapshot>>();
    const second = deferred<ApiResult<GraphSnapshot>>();
    const firstLoad = store.getState().loadGraph(first.fetcher);
    const secondLoad = store.getState().loadGraph(second.fetcher);
    expect(first.signal()?.aborted).toBe(true);
    second.settle(ready(2));
    await secondLoad;
    first.settle(ready(5));
    await firstLoad;
    expect(store.getState().load).toEqual({
      status: 'ready',
      snapshot: snapshotOf(2),
      receivedMs: NOW_MS,
    });
  });

  it('ignores a response that arrives after the load was cancelled', async () => {
    const store = createMapStore();
    const pending = deferred<ApiResult<GraphSnapshot>>();
    const load = store.getState().loadGraph(pending.fetcher);
    store.getState().cancelLoad();
    expect(pending.signal()?.aborted).toBe(true);
    pending.settle(ready(4));
    await load;
    expect(store.getState().load).toEqual({ status: 'loading' });
  });
});

describe('map store: selection, hover and threshold', () => {
  it('selects a known file and refuses an unknown id or a selection before ready', async () => {
    const store = createMapStore();
    store.getState().selectFile(0);
    expect(store.getState().selectedFile).toBeNull();
    await store.getState().loadGraph(() => Promise.resolve(ready(3)));
    store.getState().selectFile(2);
    expect(store.getState().selectedFile).toBe(2);
    store.getState().selectFile(3);
    expect(store.getState().selectedFile).toBeNull();
    store.getState().selectFile(1);
    store.getState().selectFile(null);
    expect(store.getState().selectedFile).toBeNull();
  });

  it('records a fly request with each keyboard selection, numbered so a repeat still flies', async () => {
    const store = await readyStore(3);
    store.getState().selectFile(1, { fly: true });
    expect(store.getState().flyTo).toEqual({ file: 1, target: 'file', serial: 1 });
    store.getState().selectFile(1, { fly: true });
    expect(store.getState().flyTo).toEqual({ file: 1, target: 'file', serial: 2 });
    store.getState().selectFile(2);
    expect(store.getState().flyTo).toEqual({ file: 1, target: 'file', serial: 2 });
  });

  it("asks to frame a known file's folder and keeps that file selected", async () => {
    const store = await readyStore(3);
    store.getState().showFolderOf(2);
    expect(store.getState().selectedFile).toBe(2);
    expect(store.getState().flyTo).toEqual({ file: 2, target: 'folder', serial: 1 });
    store.getState().showFolderOf(3);
    expect(store.getState().flyTo).toEqual({ file: 2, target: 'folder', serial: 1 });
  });

  it('hovers a known file only, and clears hover and selection on a new load', async () => {
    const store = await readyStore(3);
    store.getState().hoverFile(4);
    expect(store.getState().hoveredFile).toBeNull();
    store.getState().hoverFile(1.5);
    expect(store.getState().hoveredFile).toBeNull();
    store.getState().hoverFile(2);
    store.getState().selectFile(1);
    expect([store.getState().hoveredFile, store.getState().selectedFile]).toEqual([2, 1]);
    await store.getState().loadGraph(() => Promise.resolve(ready(3)));
    expect([store.getState().hoveredFile, store.getState().selectedFile]).toEqual([null, null]);
  });

  it('clamps the threshold to 0 to 1 and ignores a value that is not a number', () => {
    const store = createMapStore();
    store.getState().setMinStrength(1.7);
    expect(store.getState().minStrength).toBe(1);
    store.getState().setMinStrength(-0.2);
    expect(store.getState().minStrength).toBe(0);
    store.getState().setMinStrength(0.4);
    store.getState().setMinStrength(Number.NaN);
    expect(store.getState().minStrength).toBe(0.4);
  });
});

describe('map store: summary and detail', () => {
  it('keeps the summary or its error', async () => {
    const store = createMapStore();
    await store.getState().loadSummary(() => Promise.resolve({ ok: true, value: SUMMARY }));
    expect(store.getState().summary).toEqual({ status: 'ready', summary: SUMMARY });
    const error = { code: 'network', message: 'down' } as const;
    await store.getState().loadSummary(() => Promise.resolve({ ok: false, error }));
    expect(store.getState().summary).toEqual({ status: 'error', error });
  });

  it('loads the selected file and stays idle without a selection', async () => {
    const store = await readyStore(3);
    await store.getState().loadDetail(() => Promise.reject(new Error('must not be asked')));
    expect(store.getState().detail).toEqual({ status: 'idle' });
    store.getState().selectFile(2);
    const asked: number[] = [];
    await store.getState().loadDetail((id) => {
      asked.push(id);
      return Promise.resolve({ ok: true, value: detailFor(id) });
    });
    expect(asked).toEqual([2]);
    expect(store.getState().detail).toEqual({ status: 'ready', detail: detailFor(2) });
  });

  it('refuses a detail whose path is not the selected file in the map', async () => {
    const store = await readyStore(3);
    store.getState().selectFile(1);
    await store
      .getState()
      .loadDetail((id) => Promise.resolve({ ok: true, value: detailFor(id, 'other.rs') }));
    const detail = store.getState().detail;
    expect(detail.status === 'error' ? detail.error.code : detail.status).toBe('bad_range');
  });

  it('drops a detail that arrives after the selection moved on', async () => {
    const store = await readyStore(3);
    store.getState().selectFile(0);
    const slow = deferred<ApiResult<FileDetail>>();
    const first = store.getState().loadDetail((_id, signal) => slow.fetcher(signal));
    store.getState().selectFile(1);
    const second = store
      .getState()
      .loadDetail((id) => Promise.resolve({ ok: true, value: detailFor(id) }));
    await second;
    slow.settle({ ok: true, value: detailFor(0) });
    await first;
    expect(store.getState().detail).toEqual({ status: 'ready', detail: detailFor(1) });
  });
});
