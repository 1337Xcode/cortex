// Whether Cortex has read the repository's commit history, from the daemon's health as the store
// last heard it: the page asks for it when it opens and again with each quiet refresh, so a file's
// commit count of 0 is shown as an answer only once the history miner reports it is ready.
import { historyRead, type HistoryRead } from '../text/historyWords';
import { useMapStore } from './store';

/** What the store's health says of the repository's history; not read until the health arrives. */
export function useHistoryRead(): HistoryRead {
  const health = useMapStore((state) => state.health);
  return historyRead(health.status === 'ready' ? health.value : null);
}
