// Tests for the store: each load state, selection and hover rules, the strength threshold, the
// summary and detail loads, and dropping late responses.
import { describe, expect, it } from 'vitest';
import type { FileDetail } from '../api/file';
import type { GraphSnapshot } from '../api/graph';
import type { ApiResult } from '../api/guard';
import type { Health } from '../api/health';
import type { Receipts } from '../api/receipts';
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
    commits_counted: true,
  }));
  return { version: 1, files, links: [] };
}

function ready(count: number): ApiResult<GraphSnapshot> {
  return { ok: true, value: snapshotOf(count) };
}

function detailFor(id: number, path = `f${String(id).padStart(3, '0')}.rs`): FileDetail {
  return {
    version: 1,
    id,
    path,
    facts: [],
    history: [],
    history_total: 0,
    history_reader: 'on',
    symbols: [],
    symbols_total: 0,
    selected: [],
    told: [],
    editor_url: null,
  };
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
  history: { max_commits: 5000, max_age_days: 365, max_files_per_commit: 30 },
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

  it('asks to frame a folder files sit under, keeps the selection, and ignores others', async () => {
    const store = createMapStore(() => NOW_MS);
    const nested = snapshotOf(2);
    const files = [
      { ...nested.files[0]!, path: 'src/a.rs' },
      { ...nested.files[1]!, path: 'src/b/c.rs' },
    ];
    await store
      .getState()
      .loadGraph(() => Promise.resolve({ ok: true, value: { ...nested, files } }));
    store.getState().selectFile(1);
    store.getState().showFolder('src/b');
    expect(store.getState().selectedFile).toBe(1);
    expect(store.getState().flyTo).toEqual({ target: 'folder', folder: 'src/b', serial: 1 });
    // "src/a" is a file's path without its extension, not a folder; "sr" is only a prefix.
    store.getState().showFolder('src/a');
    store.getState().showFolder('sr');
    expect(store.getState().flyTo).toEqual({ target: 'folder', folder: 'src/b', serial: 1 });
    store.getState().showFolder('');
    expect(store.getState().flyTo).toEqual({ target: 'folder', folder: '', serial: 2 });
  });

  it('keeps the empty state while asking again quietly, and shows files once they arrive', async () => {
    const store = createMapStore(() => NOW_MS);
    await store.getState().loadGraph(() => Promise.resolve(ready(0)));
    expect(store.getState().load.status).toBe('empty');
    const later = deferred<ApiResult<GraphSnapshot>>();
    const asking = store.getState().loadGraph(later.fetcher, { isQuiet: true });
    expect(store.getState().load.status).toBe('empty');
    later.settle(ready(2));
    await asking;
    expect(store.getState().load.status).toBe('ready');
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

  it('asks for no receipts until told, then keeps them or their error, dropping a late answer', async () => {
    const store = createMapStore();
    expect(store.getState().receipts).toEqual({ status: 'idle' });
    const receipts: Receipts = {
      version: 1,
      window_days: 7,
      prices: { state: 'missing', why: 'no home folder' },
      recorded: {
        holdout: { held_out: 0, treated: 0, needed_per_arm: 30, comparison: null },
        precision: { labelled: 0, used: 0 },
        warnings: { labelled: 0, used: 0 },
        sessions: [],
      },
    };
    const slow = deferred<ApiResult<Receipts>>();
    const first = store.getState().loadReceipts((signal) => slow.fetcher(signal));
    expect(store.getState().receipts).toEqual({ status: 'loading' });
    const error = { code: 'http_status', message: 'the daemon answered 500' } as const;
    await store.getState().loadReceipts(() => Promise.resolve({ ok: false, error }));
    slow.settle({ ok: true, value: receipts });
    await first;
    expect(store.getState().receipts).toEqual({ status: 'error', error });
    await store.getState().loadReceipts(() => Promise.resolve({ ok: true, value: receipts }));
    expect(store.getState().receipts).toEqual({ status: 'ready', receipts });
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
    // The daemon's ids moved on: one quiet refresh is asked for, once for this snapshot.
    expect(store.getState().refreshSerial).toBe(1);
    await store
      .getState()
      .loadDetail((id) => Promise.resolve({ ok: true, value: detailFor(id, 'other.rs') }));
    expect(store.getState().refreshSerial).toBe(1);
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

describe('map store: views, live state and the first reading', () => {
  it('opens on the default view, switches to a known view and ignores an unknown name', () => {
    const store = createMapStore(() => NOW_MS);
    expect(store.getState().view).toBe('default');
    store.getState().setView('heat');
    expect(store.getState().view).toBe('heat');
    store.getState().setView('nonsense');
    expect(store.getState().view).toBe('heat');
  });

  it('connects, takes each frame whole, and pauses without touching the selection', async () => {
    const store = await readyStore(3);
    store.getState().selectFile(1);
    expect(store.getState().live).toEqual({ status: 'connecting' });
    const frame = {
      version: 1 as const,
      sessions: [{ id: 'a', agent: 'claude', title: 'Claude Code', files: [] }],
    };
    store.getState().takeLive(frame);
    expect(store.getState().live).toEqual({ status: 'open', state: frame });
    store.getState().pauseLive();
    expect(store.getState().live).toEqual({ status: 'paused' });
    expect([store.getState().selectedFile, store.getState().flyTo]).toEqual([1, null]);
  });

  it('keeps what the health says of the first reading, or its error', async () => {
    const store = createMapStore(() => NOW_MS);
    expect(store.getState().index).toEqual({ status: 'unknown' });
    const health: Health = {
      version: '0.4.0',
      has_assets: true,
      index: 'reading',
      daemon: { status: 'absent' },
    };
    await store.getState().loadHealth(() => Promise.resolve({ ok: true, value: health }));
    expect(store.getState().index).toEqual({ status: 'reading' });
    const error = { code: 'network' as const, message: 'down' };
    await store.getState().loadHealth(() => Promise.resolve({ ok: false, error }));
    expect(store.getState().index).toEqual({ status: 'error', error });
  });

  it('keeps a known first reading when an answer cannot be read, and fails an unknown one', async () => {
    const store = createMapStore(() => NOW_MS);
    const refused = { code: 'bad_range' as const, message: 'health.version: too long' };
    await store.getState().loadHealth(() => Promise.resolve({ ok: false, error: refused }));
    expect(store.getState().index).toEqual({ status: 'error', error: refused });
    const reading: Health = {
      version: '0.4.0',
      has_assets: true,
      index: 'reading',
      daemon: { status: 'unreadable', error: refused },
    };
    // A report the page cannot read is the health section's to show; the empty map still reads.
    await store.getState().loadHealth(() => Promise.resolve({ ok: true, value: reading }));
    expect(store.getState().index).toEqual({ status: 'reading' });
    await store.getState().loadHealth(() => Promise.resolve({ ok: false, error: refused }));
    expect(store.getState().index).toEqual({ status: 'reading' });
    const health = store.getState().health;
    expect(health.status === 'error' ? health.error : health.status).toEqual(refused);
  });
});
