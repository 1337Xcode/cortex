// What every part of the store shares: one request slot per kind of request (a newer start or a
// cancel makes every older start of that kind stale), the store's own `set` and `get`, and its
// clock. The store is built from three parts (see `store.ts`), each given this context.
import type { StoreApi } from 'zustand/vanilla';
import type { MapState } from './store';

/** One kind of request: a newer start or a cancel makes every older start stale. */
export interface RequestSlot {
  readonly begin: () => { readonly signal: AbortSignal; readonly isCurrent: () => boolean };
  readonly cancel: () => void;
}

/** A request slot with no request in flight. */
export function requestSlot(): RequestSlot {
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

/** The store's request slots, one per endpoint it asks. */
export interface StoreRequests {
  readonly graph: RequestSlot;
  readonly summary: RequestSlot;
  readonly receipts: RequestSlot;
  readonly detail: RequestSlot;
  readonly health: RequestSlot;
  readonly activity: RequestSlot;
  readonly warnings: RequestSlot;
}

/** A slot for each of the store's requests, none in flight. */
export function storeRequests(): StoreRequests {
  return {
    graph: requestSlot(),
    summary: requestSlot(),
    receipts: requestSlot(),
    detail: requestSlot(),
    health: requestSlot(),
    activity: requestSlot(),
    warnings: requestSlot(),
  };
}

/** Cancels every request of `requests` in flight; their answers are dropped. */
export function cancelEvery(requests: StoreRequests): void {
  const { graph, summary, receipts, detail, health, activity, warnings } = requests;
  for (const slot of [graph, summary, receipts, detail, health, activity, warnings]) slot.cancel();
}

/** Whether a load shows its loading state (a first load) or keeps what is shown until it answers. */
export interface LoadOptions {
  readonly isQuiet: boolean;
}

/** What a part of the store is built with. */
export interface SliceContext {
  readonly set: StoreApi<MapState>['setState'];
  readonly get: () => MapState;
  readonly requests: StoreRequests;
  /** The store's clock, in ms since the epoch; tests give a fixed one. */
  readonly clockMs: () => number;
}
