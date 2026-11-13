// Tests for the receipts panel's words. Most expected lines are the ones `cortex receipts` prints
// for the same numbers (its own tests' cases, so the two say the same); the rest are worked by hand
// in the comments beside them.
import { describe, expect, it } from 'vitest';
import type { Compared, Holdout, SessionReceipt, Tally, TaskReceipt } from '../api/receipts';
import {
  amount,
  headline,
  precisionSentence,
  pricesSentence,
  quantity,
  sessionOverhead,
  sessionStarted,
  sessionTotal,
  share,
  signedShare,
  taskSentence,
  twoPlaces,
  usageWords,
  warningSentence,
  when,
} from './receiptWords';

/** 2000-03-01 is 11,017 days after 1970-01-01, and 3,723,004 ms is 01:02:03.004. */
const MARCH_FIRST_2000_0102_MS = 11_017 * 86_400_000 + 3_723_004;

function measure(treated: number, heldOut: number, low: number, high: number): Compared {
  return { treated, held_out: heldOut, difference: treated - heldOut, low, high };
}

function tally(extra: Partial<Tally> = {}): Tally {
  return {
    turns: 3,
    cumulative_input: 1_500,
    uncached_input: 200,
    cache_reads: 1_200,
    cache_writes: 100,
    output: 90,
    estimate: {
      picodollars: 109_168_100_000,
      cache_write_picodollars: 0,
      priced_turns: 3,
      unpriced_turns: {},
    },
    ...extra,
  };
}

function task(extra: Partial<TaskReceipt> = {}): TaskReceipt {
  return {
    task_id: '01JC0000000000000000000014',
    seq: 14,
    started_at_ms: MARCH_FIRST_2000_0102_MS,
    ended_at_ms: MARCH_FIRST_2000_0102_MS + 60_000,
    outcome: 'success',
    held_out: false,
    tool_calls: 4,
    injected_tokens: 120,
    usage: tally(),
    ...extra,
  };
}

function session(extra: Partial<SessionReceipt> = {}): SessionReceipt {
  return {
    session_id: '3f2a9c1e-session',
    agent: 'claude',
    is_metered: true,
    started_at_ms: MARCH_FIRST_2000_0102_MS,
    ended_at_ms: null,
    status: 'live',
    subagents: 0,
    tool_calls: 4,
    tasks: [task()],
    tasks_total: 1,
    outside_tasks: tally({ turns: 0, cumulative_input: 0, uncached_input: 0, cache_reads: 0 }),
    outside_tool_calls: 0,
    total: tally({ cumulative_input: 153_282, uncached_input: 152_082 }),
    compactions_left_out: 0,
    cache_breaks: [],
    cache_breaks_total: 0,
    primer_tokens: 400,
    injected_tokens: 520,
    ...extra,
  };
}

describe('numbers as the CLI writes them', () => {
  it('rounds shares to two places, an exact half to the even hundredth as Rust does', () => {
    // 1/6 is 0.1666..., -1/12 is -0.0833...; 0.125 and 0.625 lie exactly halfway and go to the
    // even 0.12 and 0.62, 0.375 and 0.875 to 0.38 and 0.88; a tiny negative keeps its sign.
    const cases: [number, string][] = [
      [0.5, '0.50'],
      [1 / 6, '0.17'],
      [-1 / 12, '-0.08'],
      [0.125, '0.12'],
      [0.375, '0.38'],
      [0.625, '0.62'],
      [0.875, '0.88'],
      [-0.125, '-0.12'],
      [-0.004, '-0.00'],
    ];
    for (const [value, text] of cases) expect(twoPlaces(value), String(value)).toBe(text);
  });

  it('never writes a share or a difference of shares that is not an end as 0 or 1', () => {
    // 1 of 1,000 is 0.001 and 999 of 1,000 is 0.999: two places would round them to 0.00 and
    // 1.00. In between, two places as the CLI writes them, an exact half to the even hundredth.
    expect([0, 0.001, 0.004999, 0.005, 0.125, 0.994, 0.995, 0.999, 1].map(share)).toEqual([
      '0',
      'under 0.01',
      'under 0.01',
      '0.01',
      '0.12',
      '0.99',
      'over 0.99',
      'over 0.99',
      '1',
    ]);
    expect([0, 0.003, -0.003, 0.17, -1 / 12, 0.998, -0.998, 1, -1].map(signedShare)).toEqual([
      '0',
      'under +0.01',
      'over -0.01',
      '+0.17',
      '-0.08',
      'over +0.99',
      'under -0.99',
      '+1',
      '-1',
    ]);
  });

  it('writes medians grouped in threes with a tenth when they have one, and a sign below 0', () => {
    expect(quantity(6_500)).toBe('6,500');
    expect(quantity(12.5)).toBe('12.5');
    expect(quantity(-7.5)).toBe('-7.5');
    expect(quantity(0)).toBe('0');
    expect(quantity(-0.04)).toBe('-0');
    // 1,234,567.25 is 12,345,672.5 tenths, rounded half up to 12,345,673.
    expect(quantity(1_234_567.25)).toBe('1,234,567.3');
  });

  it('rounds picodollars to hundredths of cents, as the CLI does', () => {
    expect(amount(109_168_100_000)).toBe('about $0.1092');
    expect(amount(50_000_000)).toBe('about $0.0001');
    expect(amount(49_999_999)).toBe('less than $0.0001');
    expect(amount(1)).toBe('less than $0.0001');
    expect(amount(0)).toBe('about $0.0000');
    expect(amount(12_345_678_900_000_000)).toBe('about $12345.6789');
  });

  it('shows times to the minute in UTC', () => {
    expect(when(MARCH_FIRST_2000_0102_MS)).toBe('2000-03-01 01:02 UTC');
  });
});

describe('the headline', () => {
  const holdout = (extra: Partial<Holdout>): Holdout => ({
    held_out: 12,
    treated: 40,
    needed_per_arm: 30,
    comparison: null,
    ...extra,
  });

  it('says there is not enough data yet, with the counts against 30 and no comparison', () => {
    expect(headline(holdout({}))).toEqual({
      kind: 'not-enough',
      text: 'Not enough data yet: 12 of 30 held-out and 40 of 30 treated tasks so far.',
    });
    expect(headline(holdout({ held_out: 31, treated: 1_200 }))).toEqual({
      kind: 'not-enough',
      text: 'Not enough data yet: 31 held-out and 1,200 treated tasks so far, but fewer than 30 of each have ended.',
    });
  });

  it('never states a share, a sign or a saving below 30 a side', () => {
    for (let heldOut = 0; heldOut < 30; heldOut += 1) {
      const said = headline(holdout({ held_out: heldOut, treated: 29 + heldOut }));
      expect(said.kind).toBe('not-enough');
      expect(said.text).not.toMatch(/%|\+|sav|interval|differ/i);
    }
  });

  it('lists each measure as the CLI does, marking the intervals that hold 0', () => {
    const said = headline(
      holdout({
        held_out: 31,
        comparison: {
          tasks: [40, 31],
          success_rate: measure(0.5, 1 / 3, -1 / 12, 0.4),
          per_success_tasks: [30, 30],
          per_success: [
            measure(800, 6_500, -9_000, -2_100),
            measure(12.5, 20, -15, 0),
            measure(3, 4, -2, 0.5),
            measure(0, 0, 0, 0),
          ],
        },
      }),
    );
    expect(said).toEqual({
      kind: 'compared',
      text: 'Holdout comparison: 40 tasks ended with Cortex and 31 held out; each difference is with Cortex minus held out, its 95% bootstrap interval in brackets.',
      measures: [
        {
          name: 'Success rate',
          arms: '0.50 with Cortex, 0.33 held out',
          difference: 'difference +0.17 (-0.08 to +0.40)',
          holdsZero: true,
        },
        {
          name: 'Median input tokens per successful task',
          arms: '800 with Cortex, 6,500 held out',
          difference: 'difference -5,700 (-9,000 to -2,100)',
          holdsZero: false,
        },
        {
          name: 'Median uncached input tokens per successful task',
          arms: '12.5 with Cortex, 20 held out',
          difference: 'difference -7.5 (-15 to 0)',
          holdsZero: true,
        },
        {
          name: 'Median turns per successful task',
          arms: '3 with Cortex, 4 held out',
          difference: 'difference -1 (-2 to +0.5)',
          holdsZero: true,
        },
        {
          name: 'Median tool calls per successful task',
          arms: '0 with Cortex, 0 held out',
          difference: 'difference 0 (0 to 0)',
          holdsZero: true,
        },
      ],
      medians: 'Medians from 30 successful tasks with Cortex and 30 held out whose usage was read.',
    });
  });

  it('says how far the medians are from enough when they are not given', () => {
    const said = headline(
      holdout({
        held_out: 31,
        comparison: {
          tasks: [40, 31],
          success_rate: measure(0.5, 0.5, -0.1, 0.1),
          per_success_tasks: [12, 30],
          per_success: null,
        },
      }),
    );
    expect(said.kind === 'compared' ? [said.measures.length, said.medians] : said).toEqual([
      1,
      'Per successful task: 12 of 30 with Cortex and 30 of 30 held out have usage read so far.',
    ]);
  });
});

describe('precision and prices', () => {
  it('never words a precision that is not all or none as 1.00 or 0.00', () => {
    // 999 of 1,000 is 0.999 and 1 of 1,000 is 0.001; 4 of 4 is all of them.
    expect(precisionSentence({ labelled: 1000, used: 999 })).toBe(
      'Injection precision: over 0.99, 999 of 1,000 work orders given and labelled were used.',
    );
    expect(warningSentence({ labelled: 1000, used: 1 })).toBe(
      'Warning precision: under 0.01, 1 of 1,000 files warnings named and marked were then read or edited.',
    );
    expect(precisionSentence({ labelled: 4, used: 4 })).toBe(
      'Injection precision: 1, 4 of 4 work orders given and labelled were used.',
    );
  });

  it('words precision as the CLI does: 52 of 73 is 0.71, 5 of 9 is 0.56', () => {
    expect(precisionSentence({ labelled: 73, used: 52 })).toBe(
      'Injection precision: 0.71, 52 of 73 work orders given and labelled were used.',
    );
    expect(warningSentence({ labelled: 9, used: 5 })).toBe(
      'Warning precision: 0.56, 5 of 9 files warnings named and marked were then read or edited.',
    );
    expect(precisionSentence({ labelled: 0, used: 0 })).toBe(
      'Injection precision: no work order given is labelled yet.',
    );
    expect(warningSentence({ labelled: 0, used: 0 })).toBe(
      'Warning precision: no warning is marked yet.',
    );
  });

  it('names the price table every cost is estimated from, or why there is none', () => {
    expect(pricesSentence({ state: 'read', updated: '2026-11-01' })).toBe(
      'Prices: as of 2026-11-01, from your price file. Every cost is an estimate.',
    );
    expect(pricesSentence({ state: 'shipped', updated: '2026-11-01' })).toBe(
      'Prices: as of 2026-11-01, from the table Cortex ships, as you have no price file. Every cost is an estimate.',
    );
    expect(pricesSentence({ state: 'missing', why: 'no home folder' })).toBe(
      'Prices: none, so no cost is estimated (no home folder).',
    );
  });
});

describe('tasks and sessions', () => {
  it('words usage as the CLI does, with why responses have no price', () => {
    const unpriced = tally({
      turns: 2,
      cumulative_input: 14,
      uncached_input: 14,
      cache_reads: 0,
      cache_writes: 0,
      output: 2,
      estimate: {
        picodollars: 0,
        cache_write_picodollars: 0,
        priced_turns: 0,
        unpriced_turns: { model: 2 },
      },
    });
    expect(usageWords(unpriced)).toBe(
      '2 turns, 14 input tokens (14 uncached, 0 read from the cache, 0 written to it), 2 output tokens, no cost estimated (2 with no price for their model)',
    );
    expect(usageWords(tally({ turns: 0 }))).toBe('no usage recorded');
  });

  it('says what a task took in one sentence with its numbers', () => {
    // 1,500 input tokens are 200 uncached + 1,200 read + 100 written; 109,168,100,000
    // picodollars round to $0.1092.
    expect(taskSentence(task(), true)).toBe(
      'Task 14 (success), started 2000-03-01 01:02 UTC: 3 turns, 1,500 input tokens (200 uncached, 1,200 read from the cache, 100 written to it), 90 output tokens, about $0.1092; 4 tool calls; 120 tokens injected by Cortex.',
    );
    expect(taskSentence(task({ held_out: true, injected_tokens: 0 }), false)).toBe(
      'Task 14 (success, held out), started 2000-03-01 01:02 UTC: 4 tool calls; 0 tokens injected by Cortex.',
    );
    expect(taskSentence(task({ outcome: null, ended_at_ms: null }), false)).toContain(
      'Task 14 (open)',
    );
  });

  it("says Cortex's overhead against the session's input, and leaves out usage it does not read", () => {
    expect(sessionOverhead(session())).toBe(
      '520 tokens injected by Cortex (primer: 400 tokens), against 153,282 input tokens in total.',
    );
    expect(sessionTotal(session())).toBe(
      'Total: 3 turns, 90 output tokens, about $0.1092; 4 tool calls.',
    );
    const codex = session({
      agent: 'codex',
      is_metered: false,
      primer_tokens: 0,
      injected_tokens: 0,
    });
    expect(sessionOverhead(codex)).toBe('0 tokens injected by Cortex.');
    expect(sessionTotal(codex)).toBe(
      "Total: 4 tool calls. Cortex does not read this agent's transcripts, so its usage is not shown.",
    );
  });

  it('counts every task of a session, not only the newest the daemon lists', () => {
    // The daemon lists the newest 200 of 250 tasks; the session's line says 250.
    const long = session({ tasks: Array.from({ length: 200 }, () => task()), tasks_total: 250 });
    expect(sessionStarted(long)).toBe('Started 2000-03-01 01:02 UTC, 250 tasks.');
    expect(sessionStarted(session({ subagents: 2 }))).toBe(
      'Started 2000-03-01 01:02 UTC, 1 task, 2 subagents.',
    );
  });
});
