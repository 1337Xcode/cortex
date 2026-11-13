// Tests for the receipts guard: a report with a comparison and one without are accepted whole,
// then one rejected input per rule of the contract, each with the literal error code it must give.
// The sums in the bodies are worked by hand in the comments beside them.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApiErrorCode } from './guard';
import {
  fetchReceipts,
  MAX_CACHE_BREAKS_SHOWN,
  MAX_RECEIPT_SESSIONS,
  MAX_TASKS_SHOWN,
  parseReceipts,
} from './receipts';

type Body = Record<string, unknown>;

const PRICED = { picodollars: 0, cache_write_picodollars: 0, priced_turns: 0, unpriced_turns: {} };

/** A tally whose cumulative input is its three parts: `uncached` + `reads` + `writes`. */
function tally(turns: number, uncached: number, reads: number, writes: number, extra: Body = {}) {
  return {
    turns,
    cumulative_input: uncached + reads + writes,
    uncached_input: uncached,
    cache_reads: reads,
    cache_writes: writes,
    output: 10 * turns,
    estimate: { ...PRICED, priced_turns: turns, picodollars: 1_000_000 * turns },
    ...extra,
  };
}

function task(seq: number, startedMs: number, turns: number, extra: Body = {}): Body {
  return {
    task_id: `01JC0000000000000000000${String(seq).padStart(3, '0')}`,
    seq,
    started_at_ms: startedMs,
    ended_at_ms: startedMs + 60_000,
    outcome: 'success',
    held_out: false,
    tool_calls: 2,
    injected_tokens: 30,
    usage: tally(turns, 100, 200, 0),
    ...extra,
  };
}

/**
 * One session of two tasks: 3 and 2 turns, 1 turn outside them, 6 in total; input 300 a task
 * (100 + 200 + 0) and 50 outside, 650 in total (250 + 400 + 0); 2 tool calls a task and 1
 * outside, 5 in total; 30 injected tokens a task and a 100-token primer, 160 in total. Its totals
 * of tasks and cache breaks are those listed, unless `extra` sets them.
 */
function session(extra: Body = {}): Body {
  const body = sessionBody(extra);
  const { tasks, cache_breaks: breaks } = body;
  return {
    tasks_total: Array.isArray(tasks) ? tasks.length : 0,
    cache_breaks_total: Array.isArray(breaks) ? breaks.length : 0,
    ...body,
  };
}

/**
 * A session of 250 tasks of which the daemon lists the newest 200 (numbers 51 to 250), each of 1
 * turn, 2 tool calls and 30 injected tokens, and 260 cache breaks of which it lists 200. Every
 * task counts in the totals: 250 turns and 1 outside them, 251; 500 tool calls and 1 outside,
 * 501; 7,500 injected tokens and a 100-token primer, 7,600.
 */
function cutSession(extra: Body = {}): Body {
  return session({
    tasks: Array.from({ length: 200 }, (_, i) => task(51 + i, 1_100_000 + i * 1000, 1)),
    tasks_total: 250,
    total: tally(251, 25_050, 50_000, 0),
    tool_calls: 501,
    injected_tokens: 7600,
    cache_breaks: Array.from({ length: 200 }, (_, i) => ({
      session_id: 'sess-1',
      turn: i + 1,
      at_ms: 1_600_000 + i,
      cause: null,
    })),
    cache_breaks_total: 260,
    ...extra,
  });
}

/** The fields of `session`, before its totals of tasks and cache breaks. */
function sessionBody(extra: Body): Body {
  return {
    session_id: 'sess-1',
    agent: 'claude',
    is_metered: true,
    started_at_ms: 1_000_000,
    ended_at_ms: 2_000_000,
    status: 'ended',
    subagents: 0,
    tool_calls: 5,
    tasks: [task(1, 1_100_000, 3), task(2, 1_500_000, 2)],
    outside_tasks: tally(1, 50, 0, 0),
    outside_tool_calls: 1,
    total: tally(6, 250, 400, 0),
    compactions_left_out: 0,
    cache_breaks: [{ session_id: 'sess-1', turn: 4, at_ms: 1_600_000, cause: 'idle_gap' }],
    primer_tokens: 100,
    injected_tokens: 160,
    ...extra,
  };
}

/** A measure where the difference is with Cortex minus held out: 0.5 - 0.25 is 0.25. */
function measure(treated: number, heldOut: number, low: number, high: number): Body {
  return { treated, held_out: heldOut, difference: treated - heldOut, low, high };
}

const COMPARISON: Body = {
  tasks: [40, 31],
  success_rate: measure(0.5, 0.25, -0.05, 0.5),
  per_success_tasks: [30, 30],
  per_success: [
    measure(800, 6500, -9000, -2100),
    measure(12.5, 20, -15, 0),
    measure(3, 4, -2, 0.5),
    measure(0, 0, 0, 0),
  ],
};

function receipts(recorded: Body = {}, extra: Body = {}): Body {
  return {
    version: 1,
    window_days: 7,
    prices: { state: 'shipped', updated: '2026-11-01' },
    recorded: {
      holdout: { held_out: 31, treated: 40, needed_per_arm: 30, comparison: COMPARISON },
      precision: { labelled: 73, used: 52 },
      warnings: { labelled: 9, used: 5 },
      sessions: [session()],
      ...recorded,
    },
    ...extra,
  };
}

/** The body with `holdout` in place of the default arms. */
function withHoldout(holdout: Body): Body {
  return receipts({ holdout: { held_out: 31, treated: 40, needed_per_arm: 30, ...holdout } });
}

describe('parseReceipts accepts', () => {
  it('a report with a comparison, keeping every field', () => {
    expect(parseReceipts(receipts())).toEqual({ ok: true, value: receipts() });
  });

  it('too few tasks to compare, no prices and no session', () => {
    const cold = receipts(
      {
        holdout: { held_out: 12, treated: 40, needed_per_arm: 30, comparison: null },
        sessions: [],
      },
      { prices: { state: 'missing', why: 'no home folder' } },
    );
    expect(parseReceipts(cold)).toEqual({ ok: true, value: cold });
  });

  it('a session whose usage is not read, with nothing in its tallies', () => {
    const none = tally(0, 0, 0, 0, { output: 0, estimate: { ...PRICED } });
    const codex = session({
      agent: 'codex',
      is_metered: false,
      tasks: [task(1, 1_100_000, 0, { usage: none })],
      outside_tasks: none,
      total: none,
      tool_calls: 3,
      cache_breaks: [],
    });
    expect(parseReceipts(receipts({ sessions: [codex] })).ok).toBe(true);
  });

  it('a session cut to its newest 200 tasks and cache breaks, its totals counting every one', () => {
    const cut = receipts({ sessions: [cutSession()] });
    expect(parseReceipts(cut)).toEqual({ ok: true, value: cut });
  });

  it('an amount past 2^53 picodollars, which arrives whole but rounded', () => {
    const big = tally(6, 250, 400, 0, {
      estimate: { ...PRICED, priced_turns: 6, picodollars: 2 ** 60 },
    });
    expect(parseReceipts(receipts({ sessions: [session({ total: big })] })).ok).toBe(true);
  });
});

/** The body with the first session's first task changed by `extra`. */
function withTask(extra: Body): Body {
  return receipts({
    sessions: [session({ tasks: [task(1, 1_100_000, 3, extra), task(2, 1_500_000, 2)] })],
  });
}

const REJECTED: [string, unknown, ApiErrorCode][] = [
  ['null', null, 'bad_shape'],
  ['an unknown version', receipts({}, { version: 2 }), 'bad_version'],
  ['a window of 0 days', receipts({}, { window_days: 0 }), 'bad_range'],
  ['an unknown price state', receipts({}, { prices: { state: 'guessed' } }), 'bad_kind'],
  [
    'a price day that is no day',
    receipts({}, { prices: { state: 'read', updated: 'today' } }),
    'bad_shape',
  ],
  [
    'a missing price table without a why',
    receipts({}, { prices: { state: 'missing' } }),
    'bad_shape',
  ],
  [
    'an estimate when no table was read',
    receipts({}, { prices: { state: 'missing', why: 'none' } }),
    'bad_range',
  ],
  ['a negative arm', withHoldout({ held_out: -1 }), 'bad_shape'],
  ['a fractional arm', withHoldout({ treated: 40.5 }), 'bad_shape'],
  ['no task needed per arm', withHoldout({ needed_per_arm: 0, comparison: null }), 'bad_range'],
  ['a comparison at 29 held out', withHoldout({ held_out: 29 }), 'bad_range'],
  [
    'a comparison of fewer ended tasks than needed',
    withHoldout({ comparison: { ...COMPARISON, tasks: [40, 29] } }),
    'bad_range',
  ],
  [
    'a comparison of more tasks than an arm holds',
    withHoldout({ comparison: { ...COMPARISON, tasks: [41, 31] } }),
    'bad_range',
  ],
  [
    'medians per successful task from 29',
    withHoldout({ comparison: { ...COMPARISON, per_success_tasks: [29, 30] } }),
    'bad_range',
  ],
  [
    'no medians with 30 a side',
    withHoldout({ comparison: { ...COMPARISON, per_success: null } }),
    'bad_range',
  ],
  [
    'three medians',
    withHoldout({ comparison: { ...COMPARISON, per_success: [measure(1, 1, 0, 0)] } }),
    'bad_shape',
  ],
  [
    'a difference that is not with Cortex minus held out',
    withHoldout({
      comparison: {
        ...COMPARISON,
        success_rate: { ...measure(0.5, 0.25, 0, 0.5), difference: -0.25 },
      },
    }),
    'bad_range',
  ],
  [
    'an interval whose low end is above its high end',
    withHoldout({ comparison: { ...COMPARISON, success_rate: measure(0.5, 0.25, 0.4, 0.1) } }),
    'bad_range',
  ],
  [
    'a success rate above 1',
    withHoldout({ comparison: { ...COMPARISON, success_rate: measure(1.5, 0.25, 0, 0.5) } }),
    'bad_range',
  ],
  [
    'a difference that is not a number',
    withHoldout({
      comparison: { ...COMPARISON, success_rate: { ...measure(0.5, 0.25, 0, 0.5), low: 'NaN' } },
    }),
    'bad_shape',
  ],
  [
    'more work orders used than labelled',
    receipts({ precision: { labelled: 3, used: 4 } }),
    'bad_range',
  ],
  ['a negative warning count', receipts({ warnings: { labelled: -1, used: 0 } }), 'bad_shape'],
  [
    'an agent that is no snake name',
    receipts({ sessions: [session({ agent: 'Claude Code' })] }),
    'bad_shape',
  ],
  [
    'a session id with a line break',
    receipts({ sessions: [session({ session_id: 'a\nb' })] }),
    'bad_shape',
  ],
  ['an unknown status', receipts({ sessions: [session({ status: 'sleeping' })] }), 'bad_kind'],
  [
    'sessions out of order',
    receipts({ sessions: [session({ started_at_ms: 3_000_000 }), session()] }),
    'bad_order',
  ],
  [
    'tasks out of order',
    receipts({ sessions: [session({ tasks: [task(2, 1_500_000, 2), task(1, 1_100_000, 3)] })] }),
    'bad_order',
  ],
  ['a task id that is no ULID', withTask({ task_id: 'task-1' }), 'bad_shape'],
  ['an unknown outcome', withTask({ outcome: 'great' }), 'bad_kind'],
  ['held out as a string', withTask({ held_out: 'yes' }), 'bad_shape'],
  ['a negative token count', withTask({ injected_tokens: -5 }), 'bad_shape'],
  [
    'an input that is not its parts added up',
    withTask({ usage: { ...tally(3, 100, 200, 0), cumulative_input: 301 } }),
    'bad_range',
  ],
  [
    'priced and unpriced responses that are not the turns',
    withTask({ usage: tally(3, 100, 200, 0, { estimate: { ...PRICED, priced_turns: 2 } }) }),
    'bad_range',
  ],
  [
    'an unknown reason a response has no price',
    withTask({
      usage: tally(3, 100, 200, 0, {
        estimate: { ...PRICED, priced_turns: 2, unpriced_turns: { weather: 1 } },
      }),
    }),
    'bad_kind',
  ],
  [
    'cache writes that cost more than the whole',
    withTask({
      usage: tally(3, 100, 200, 0, {
        estimate: { ...PRICED, priced_turns: 3, picodollars: 5, cache_write_picodollars: 6 },
      }),
    }),
    'bad_range',
  ],
  [
    'a fractional amount',
    withTask({
      usage: tally(3, 100, 200, 0, { estimate: { ...PRICED, priced_turns: 3, picodollars: 0.5 } }),
    }),
    'bad_shape',
  ],
  [
    'a total of more turns than its tasks and the rest',
    receipts({ sessions: [session({ total: tally(7, 250, 400, 0) })] }),
    'bad_range',
  ],
  [
    'tool calls that do not add up',
    receipts({ sessions: [session({ tool_calls: 6 })] }),
    'bad_range',
  ],
  [
    'fewer injected tokens than the primer and the tasks',
    receipts({ sessions: [session({ injected_tokens: 159 })] }),
    'bad_range',
  ],
  [
    'usage of a session whose transcripts are not read',
    receipts({ sessions: [session({ is_metered: false })] }),
    'bad_range',
  ],
  [
    'an unknown cause of a cache break',
    receipts({
      sessions: [
        session({ cache_breaks: [{ session_id: 's', turn: 1, at_ms: 1, cause: 'luck' }] }),
      ],
    }),
    'bad_kind',
  ],
  [
    `more than ${MAX_RECEIPT_SESSIONS} sessions`,
    receipts({ sessions: Array.from({ length: MAX_RECEIPT_SESSIONS + 1 }, () => session()) }),
    'bad_range',
  ],
  [
    `more than ${MAX_TASKS_SHOWN} tasks in a session`,
    receipts({
      sessions: [session({ tasks: Array.from({ length: MAX_TASKS_SHOWN + 1 }, () => ({})) })],
    }),
    'bad_range',
  ],
  [
    `more than ${MAX_CACHE_BREAKS_SHOWN} cache breaks in a session`,
    receipts({
      sessions: [
        session({
          cache_breaks: Array.from({ length: MAX_CACHE_BREAKS_SHOWN + 1 }, () => ({})),
        }),
      ],
    }),
    'bad_range',
  ],
  [
    'a total of tasks below those listed',
    receipts({ sessions: [session({ tasks_total: 1 })] }),
    'bad_range',
  ],
  [
    'a total of tasks above those listed while there is room for them',
    receipts({ sessions: [session({ tasks_total: 3 })] }),
    'bad_range',
  ],
  [
    'a missing total of tasks',
    receipts({ sessions: [session({ tasks_total: undefined })] }),
    'bad_shape',
  ],
  [
    'a total of cache breaks below those listed',
    receipts({ sessions: [session({ cache_breaks_total: 0 })] }),
    'bad_range',
  ],
  [
    'a total of cache breaks above those listed while there is room for them',
    receipts({ sessions: [session({ cache_breaks_total: 2 })] }),
    'bad_range',
  ],
  [
    'a cut session whose listed tasks take more turns than its total',
    receipts({ sessions: [cutSession({ total: tally(200, 25_050, 50_000, 0) })] }),
    'bad_range',
  ],
  [
    'a cut session whose listed tasks make more tool calls than its total',
    receipts({ sessions: [cutSession({ tool_calls: 400 })] }),
    'bad_range',
  ],
];

describe('parseReceipts rejects', () => {
  it.each(REJECTED)('%s', (_name, input, code) => {
    const result = parseReceipts(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});

describe('fetchReceipts', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('asks the relative URL api/receipts and parses the body', async () => {
    const asked: string[] = [];
    vi.stubGlobal('fetch', (url: string) => {
      asked.push(url);
      return Promise.resolve(new Response(JSON.stringify(receipts())));
    });
    const result = await fetchReceipts(new AbortController().signal);
    expect(asked).toEqual(['api/receipts']);
    expect(result.ok).toBe(true);
  });
});
