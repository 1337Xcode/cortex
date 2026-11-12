// Tests for the store: each load state, selection rules, and dropping late responses.
import { describe, expect, it } from 'vitest';
import type { GraphResult, GraphSnapshot } from '../api/graph';
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
  }));
  return { version: 1, files, links: [] };
}

function ready(count: number): GraphResult {
  return { ok: true, snapshot: snapshotOf(count) };
}

/** A fetcher the test settles by hand, to order responses exactly. */
function deferred(): {
  fetcher: (signal: AbortSignal) => Promise<GraphResult>;
  settle: (r: GraphResult) => void;
  signal: () => AbortSignal | null;
} {
  let settle: (result: GraphResult) => void = () => undefined;
  let seen: AbortSignal | null = null;
  const fetcher = (signal: AbortSignal): Promise<GraphResult> => {
    seen = signal;
    return new Promise((resolve) => {
      settle = resolve;
    });
  };
  return {
    fetcher,
    settle: (result) => {
      settle(result);
    },
    signal: () => seen,
  };
}

describe('map store', () => {
  it('starts loading with nothing selected', () => {
    const store = createMapStore();
    expect(store.getState().load).toEqual({ status: 'loading' });
    expect(store.getState().selectedFile).toBeNull();
  });

  it('becomes ready with the snapshot', async () => {
    const store = createMapStore();
    await store.getState().loadGraph(() => Promise.resolve(ready(3)));
    expect(store.getState().load).toEqual({ status: 'ready', snapshot: snapshotOf(3) });
  });

  it('becomes empty when the snapshot has no files', async () => {
    const store = createMapStore();
    await store.getState().loadGraph(() => Promise.resolve(ready(0)));
    expect(store.getState().load).toEqual({ status: 'empty' });
  });

  it('keeps the typed error when the load fails', async () => {
    const store = createMapStore();
    const error = { code: 'http_status', message: 'the daemon answered 500' } as const;
    await store.getState().loadGraph(() => Promise.resolve({ ok: false, error }));
    expect(store.getState().load).toEqual({ status: 'error', error });
  });

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

  it('ignores an older response that arrives after a newer request', async () => {
    const store = createMapStore();
    const first = deferred();
    const second = deferred();
    const firstLoad = store.getState().loadGraph(first.fetcher);
    const secondLoad = store.getState().loadGraph(second.fetcher);
    expect(first.signal()?.aborted).toBe(true);
    second.settle(ready(2));
    await secondLoad;
    first.settle(ready(5));
    await firstLoad;
    expect(store.getState().load).toEqual({ status: 'ready', snapshot: snapshotOf(2) });
  });

  it('ignores a response that arrives after the load was cancelled', async () => {
    const store = createMapStore();
    const pending = deferred();
    const load = store.getState().loadGraph(pending.fetcher);
    store.getState().cancelLoad();
    expect(pending.signal()?.aborted).toBe(true);
    pending.settle(ready(4));
    await load;
    expect(store.getState().load).toEqual({ status: 'loading' });
  });
});
