// The names of the sessions the page knows of, built from the store: the live sessions, the
// activity's sessions and the receipts' sessions currently loaded. One set of names serves the
// markers, the rows and the sentences, so a session is named alike everywhere on the page at once.
// The names are built once per change of the three loads and shared by every reader.
import { sessionNames, type SessionNames } from '../text/sessionNames';
import { useMapStore, type LiveStatus, type MapState } from './store';

/** The sessions of the three loads the names are built from. */
type NamedLoads = Pick<MapState, 'live' | 'activity' | 'receipts'>;

/** The ids of the live sessions of the last frame; none while the live view is not open. */
function liveIds(live: LiveStatus): string[] {
  return live.status === 'open' ? live.state.sessions.map((session) => session.id) : [];
}

/** Every session id `loads` holds: the live sessions', the events' and the receipts'. */
export function sessionIdsOf({ live, activity, receipts }: NamedLoads): string[] {
  const ids = liveIds(live);
  if (activity.status === 'ready') ids.push(...activity.value.events.map((event) => event.session));
  if (receipts.status === 'ready') {
    ids.push(...receipts.receipts.recorded.sessions.map((session) => session.session_id));
  }
  return ids;
}

/** The names of the sessions `loads` holds. */
export function namesOfSessions(loads: NamedLoads): SessionNames {
  return sessionNames(sessionIdsOf(loads));
}

/** The loads the last names were built from, and those names. */
let lastBuilt: { readonly loads: NamedLoads; readonly names: SessionNames } | null = null;

/** The names for `loads`, built again only when one of the three loads is a new value. */
function sharedNamesOf(loads: NamedLoads): SessionNames {
  const last = lastBuilt;
  const isSame =
    last !== null &&
    last.loads.live === loads.live &&
    last.loads.activity === loads.activity &&
    last.loads.receipts === loads.receipts;
  if (isSame) return last.names;
  const names = namesOfSessions(loads);
  lastBuilt = { loads, names };
  return names;
}

/** The names of the page's sessions, the same object for every reader until a load changes. */
export function useSessionNames(): SessionNames {
  const live = useMapStore((state) => state.live);
  const activity = useMapStore((state) => state.activity);
  const receipts = useMapStore((state) => state.receipts);
  return sharedNamesOf({ live, activity, receipts });
}
