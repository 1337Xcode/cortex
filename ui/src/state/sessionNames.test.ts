// Tests for the names of the page's sessions: they are built from the live sessions, the activity's
// and the receipts' together, so one session reads the same in a live row, an activity sentence and
// a receipt's title, and a session only one of them holds is still told apart from the others.
import { describe, expect, it } from 'vitest';
import type { Activity, ActivityEvent } from '../api/activity';
import type { LiveSession } from '../api/live';
import type { Receipts, SessionReceipt } from '../api/receipts';
import { eventLine } from '../text/activityWords';
import { sessionName } from '../text/liveWords';
import { sessionTitle } from '../text/receiptWords';
import { createMapStore } from './store';
import { namesOfSessions } from './sessionNames';

/** Three Codex ids begun minutes apart, so the first seven characters are shared by two. */
const LIVE_ID = '01a1d373-f6af-7e0b';
const EVENT_ONLY_ID = '01a1d375-b58f-7a41';
const RECEIPT_ONLY_ID = '3f2a9c1e-0000-1111';

const LIVE: LiveSession = { id: LIVE_ID, agent: 'codex', title: 'Codex', files: [] };

function eventOf(session: string): ActivityEvent {
  return {
    at_ms: 1_762_900_000_000,
    kind: 'prompt',
    session,
    agent: { name: 'codex', title: 'Codex' },
    file: null,
    path: null,
  };
}

/** A receipt of `session`; only its id and agent are read here. */
function receiptOf(session: string): SessionReceipt {
  return { session_id: session, agent: 'codex', status: 'ended' } as SessionReceipt;
}

/** A store holding a live frame, an activity and receipts, each naming sessions of its own. */
async function loadedStore() {
  const store = createMapStore();
  store.getState().takeLive({ version: 1, sessions: [LIVE] });
  const activity: Activity = {
    version: 1,
    events: [eventOf(LIVE_ID), eventOf(EVENT_ONLY_ID)],
  };
  await store.getState().loadActivity(() => Promise.resolve({ ok: true, value: activity }));
  const receipts = {
    version: 1,
    window_days: 7,
    prices: { state: 'missing', why: 'no home folder' },
    recorded: {
      holdout: { held_out: 0, treated: 0, needed_per_arm: 30, comparison: null },
      precision: { labelled: 0, used: 0 },
      warnings: { labelled: 0, used: 0 },
      sessions: [receiptOf(LIVE_ID), receiptOf(RECEIPT_ONLY_ID)],
    },
  } as Receipts;
  await store.getState().loadReceipts(() => Promise.resolve({ ok: true, value: receipts }));
  return store;
}

describe('namesOfSessions', () => {
  it('names a session alike in a live row, an activity sentence and a receipt title', async () => {
    const names = namesOfSessions((await loadedStore()).getState());
    expect(sessionName(LIVE, names)).toBe('Codex 01a1d373…');
    expect(eventLine(eventOf(LIVE_ID), null, names)).toBe('Codex 01a1d373… was given a prompt');
    expect(sessionTitle(receiptOf(LIVE_ID), names)).toBe('Session 01a1d373… (codex, ended)');
  });

  it('tells a session only the activity holds apart from a live one that starts alike', async () => {
    const names = namesOfSessions((await loadedStore()).getState());
    expect(eventLine(eventOf(EVENT_ONLY_ID), null, names)).toBe(
      'Codex 01a1d375… was given a prompt',
    );
    expect(sessionTitle(receiptOf(RECEIPT_ONLY_ID), names)).toBe('Session 3f2a… (codex, ended)');
  });

  it('names every session by four characters before anything but the live frame is loaded', () => {
    const store = createMapStore();
    store.getState().takeLive({ version: 1, sessions: [LIVE] });
    expect(sessionName(LIVE, namesOfSessions(store.getState()))).toBe('Codex 01a1…');
  });
});
