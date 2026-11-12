// The receipts panel's words, in the words `cortex receipts` prints, so the map and the CLI say the
// same things the same way: how far the held-out tasks are from being enough to compare, the
// comparison once they are (each arm's value, the difference with Cortex minus held out and its 95%
// interval, unrounded into no claim), how precise the work orders and warnings were, what a task
// and a session took, what Cortex injected, and the price table every cost is estimated from.
// Numbers are formatted as the CLI formats them: grouped in threes, shares to two places, dollars
// to a hundredth of a cent.
import type {
  Compared,
  Estimate,
  Holdout,
  Precision,
  Prices,
  SessionReceipt,
  Tally,
  TaskReceipt,
  Unpriced,
} from '../api/receipts';
import { UNPRICED_REASONS } from '../api/receipts';
import { visibleText } from '../api/guard';
import { countOf, formatCount } from './format';
import { shortSessionId } from './liveWords';

/** A hundredth of a cent, the smallest amount receipts show, in picodollars. */
const HUNDREDTH_OF_A_CENT_PICODOLLARS = 100_000_000n;

/** Hundredths of a cent in a dollar. */
const HUNDREDTHS_OF_CENTS_PER_DOLLAR = 10_000n;

/** Why responses have no estimate, after their count, as the CLI says it. */
const UNPRICED_WORDS: Readonly<Record<Unpriced, string>> = {
  model: 'with no price for their model',
  tier: 'not at the standard tier',
  speed: 'with no price for their speed',
  region: 'with no price where they ran',
  cache_split: 'whose cache writes are not split by lifetime',
};

/** The four medians per successful task, in the order the report gives them. */
export const MEDIAN_NAMES = [
  'Median input tokens per successful task',
  'Median uncached input tokens per successful task',
  'Median turns per successful task',
  'Median tool calls per successful task',
] as const;

/**
 * `value` to two decimal places as Rust's formatter writes it: the nearest, and on an exact tie
 * the even one. A double is exactly halfway between two hundredths only when eight times it is an
 * odd whole number (0.125, 0.375, ...), where JavaScript's `toFixed` would round up instead.
 */
export function twoPlaces(value: number): string {
  const magnitude = Math.abs(value);
  const eighths = magnitude * 8;
  if (!Number.isInteger(eighths) || eighths % 2 === 0) return value.toFixed(2);
  const below = Math.floor(magnitude * 100);
  const hundredths = below % 2 === 0 ? below : below + 1;
  const text = (hundredths / 100).toFixed(2);
  return value < 0 ? `-${text}` : text;
}

/** A share as a decimal of two places. */
function rate(value: number): string {
  return twoPlaces(value);
}

/**
 * A median or a difference of medians, grouped in threes, with its tenths when it has any (a
 * median of an even count of whole numbers can end in a half); the sign is kept even when the
 * value rounds to nothing, so "-0" still says the value was below zero.
 */
export function quantity(value: number): string {
  const sign = value < 0 ? '-' : '';
  const tenths = Math.round(Math.abs(value) * 10);
  const whole = formatCount(Math.floor(tenths / 10));
  const tenth = tenths % 10;
  return tenth === 0 ? `${sign}${whole}` : `${sign}${whole}.${tenth}`;
}

/** `text`, the written form of `value`, with a plus sign when `value` is above zero. */
function signed(value: number, number: (value: number) => string): string {
  const text = number(value);
  return value > 0 ? `+${text}` : text;
}

/** Whether a measure's interval holds 0, so it shows no difference either way. */
export function holdsZero(compared: Compared): boolean {
  return compared.low <= 0 && compared.high >= 0;
}

/**
 * One measure of the comparison as the panel lists it, in the CLI's words split in two lines:
 * each arm's value ("0.50 with Cortex, 0.33 held out") and the difference with its interval
 * ("difference +0.17 (-0.08 to +0.40)").
 */
export interface MeasureLine {
  readonly name: string;
  readonly arms: string;
  readonly difference: string;
  readonly holdsZero: boolean;
}

/** What the headline says. */
export type Headline =
  | {
      /** Too few tasks a side to compare: no comparison, no difference, no share. */
      readonly kind: 'not-enough';
      readonly text: string;
    }
  | {
      readonly kind: 'compared';
      readonly text: string;
      readonly measures: readonly MeasureLine[];
      /** The sample the medians rest on, or how far it is from being enough. */
      readonly medians: string;
    };

/** What the panel says when a measure's interval holds 0. */
export const HOLDS_ZERO_NOTE = 'The interval holds 0: not a measured difference.';

/**
 * The headline: how far the held-out tasks are from being enough to compare with the rest, or the
 * comparison with its sample sizes once both are.
 */
export function headline(holdout: Holdout): Headline {
  const needed = holdout.needed_per_arm;
  const [heldOut, treated] = [holdout.held_out, holdout.treated];
  const comparison = holdout.comparison;
  if (heldOut < needed || treated < needed) {
    return {
      kind: 'not-enough',
      text: `Not enough data yet: ${formatCount(heldOut)} of ${formatCount(needed)} held-out and ${formatCount(treated)} of ${formatCount(needed)} treated tasks so far.`,
    };
  }
  if (comparison === null) {
    return {
      kind: 'not-enough',
      text: `Not enough data yet: ${formatCount(heldOut)} held-out and ${formatCount(treated)} treated tasks so far, but fewer than ${formatCount(needed)} of each have ended.`,
    };
  }
  const [endedWith, endedHeldOut] = comparison.tasks;
  const measures: MeasureLine[] = [
    measureLine('Success rate', comparison.success_rate, rate),
    ...(comparison.per_success ?? []).map((compared, index) =>
      measureLine(MEDIAN_NAMES[index] ?? '', compared, quantity),
    ),
  ];
  const [succeededWith, succeededHeldOut] = comparison.per_success_tasks;
  const medians =
    comparison.per_success === null
      ? `Per successful task: ${formatCount(succeededWith)} of ${formatCount(needed)} with Cortex and ${formatCount(succeededHeldOut)} of ${formatCount(needed)} held out have usage read so far.`
      : `Medians from ${countOf(succeededWith, 'successful task')} with Cortex and ${formatCount(succeededHeldOut)} held out whose usage was read.`;
  return {
    kind: 'compared',
    text: `Holdout comparison: ${countOf(endedWith, 'task')} ended with Cortex and ${formatCount(endedHeldOut)} held out; each difference is with Cortex minus held out, its 95% bootstrap interval in brackets.`,
    measures,
    medians,
  };
}

/** One measure as the panel lists it, its numbers written by `number`. */
function measureLine(
  name: string,
  compared: Compared,
  number: (value: number) => string,
): MeasureLine {
  return {
    name,
    arms: `${number(compared.treated)} with Cortex, ${number(compared.held_out)} held out`,
    difference: `difference ${signed(compared.difference, number)} (${signed(compared.low, number)} to ${signed(compared.high, number)})`,
    holdsZero: holdsZero(compared),
  };
}

/** How precise the work orders given were. */
export function precisionSentence(precision: Precision): string {
  if (precision.labelled === 0) return 'Injection precision: no work order given is labelled yet.';
  return `Injection precision: ${twoPlaces(precision.used / precision.labelled)}, ${formatCount(precision.used)} of ${countOf(precision.labelled, 'work order')} given and labelled were used.`;
}

/** How often the files warnings named were then read or edited. */
export function warningSentence(precision: Precision): string {
  if (precision.labelled === 0) return 'Warning precision: no warning is marked yet.';
  return `Warning precision: ${twoPlaces(precision.used / precision.labelled)}, ${formatCount(precision.used)} of ${countOf(precision.labelled, 'file')} warnings named and marked were then read or edited.`;
}

/** What every cost is estimated from, or why none is. */
export function pricesSentence(prices: Prices): string {
  switch (prices.state) {
    case 'read':
      return `Prices: as of ${prices.updated}, from your price file. Every cost is an estimate.`;
    case 'shipped':
      return `Prices: as of ${prices.updated}, from the table Cortex ships, as you have no price file. Every cost is an estimate.`;
    case 'missing':
      return `Prices: none, so no cost is estimated (${visibleText(prices.why)}).`;
  }
}

/**
 * An estimate of `picodollars` in words: dollars rounded half up to a hundredth of a cent, or that
 * it is less than that, so that a cost never reads as nothing.
 */
export function amount(picodollars: number): string {
  const exact = BigInt(picodollars);
  const hundredths =
    (exact + HUNDREDTH_OF_A_CENT_PICODOLLARS / 2n) / HUNDREDTH_OF_A_CENT_PICODOLLARS;
  if (exact > 0n && hundredths === 0n) return 'less than $0.0001';
  const dollars = hundredths / HUNDREDTHS_OF_CENTS_PER_DOLLAR;
  const rest = String(hundredths % HUNDREDTHS_OF_CENTS_PER_DOLLAR).padStart(4, '0');
  return `about $${dollars.toString()}.${rest}`;
}

/** The estimated cost of `turns` responses, one or more, and why any of them have none. */
function cost(estimate: Estimate, turns: number): string {
  const reasons = UNPRICED_REASONS.flatMap((reason) => {
    const count = estimate.unpriced_turns[reason];
    return count === undefined ? [] : [`${formatCount(count)} ${UNPRICED_WORDS[reason]}`];
  });
  if (estimate.priced_turns === 0) return `no cost estimated (${reasons.join(', ')})`;
  const estimated = amount(estimate.picodollars);
  if (estimate.priced_turns === turns) return estimated;
  return `${estimated} for ${formatCount(estimate.priced_turns)} of them (${reasons.join(', ')})`;
}

/** The responses of `tally` and what they cost; that none was recorded when there is none. */
export function usageWords(tally: Tally): string {
  if (tally.turns === 0) return 'no usage recorded';
  const tokens = `${countOf(tally.turns, 'turn')}, ${formatCount(tally.cumulative_input)} input tokens (${formatCount(tally.uncached_input)} uncached, ${formatCount(tally.cache_reads)} read from the cache, ${formatCount(tally.cache_writes)} written to it), ${countOf(tally.output, 'output token')}`;
  return tally.estimate === null ? tokens : `${tokens}, ${cost(tally.estimate, tally.turns)}`;
}

/** The UTC minute `epochMs` falls in, as `2026-11-03 07:12 UTC`. */
export function when(epochMs: number): string {
  const date = new Date(epochMs);
  if (Number.isNaN(date.getTime())) return `${formatCount(epochMs)} ms after 1970`;
  const two = (value: number): string => String(value).padStart(2, '0');
  const day = `${String(date.getUTCFullYear()).padStart(4, '0')}-${two(date.getUTCMonth() + 1)}-${two(date.getUTCDate())}`;
  return `${day} ${two(date.getUTCHours())}:${two(date.getUTCMinutes())} UTC`;
}

/** The tokens Cortex injected, as the CLI counts them. */
function injected(tokens: number): string {
  return `${countOf(tokens, 'token')} injected by Cortex`;
}

/**
 * What one task took, as one sentence: its number, how it ended (and whether it was held out), when
 * it started, its usage when the session's usage is read, its tool calls and what Cortex injected.
 */
export function taskSentence(task: TaskReceipt, isMetered: boolean): string {
  const state = task.outcome ?? (task.ended_at_ms === null ? 'open' : 'ended');
  const arm = task.held_out === true ? ', held out' : '';
  const parts = [
    ...(isMetered ? [usageWords(task.usage)] : []),
    countOf(task.tool_calls, 'tool call'),
    injected(task.injected_tokens),
  ];
  return `Task ${formatCount(task.seq)} (${state}${arm}), started ${when(task.started_at_ms)}: ${parts.join('; ')}.`;
}

/** A session's name and state, as its row's first line ("Session 3f2a… (claude, ended)"). */
export function sessionTitle(session: SessionReceipt): string {
  return `Session ${visibleText(shortSessionId(session.session_id))} (${session.agent}, ${session.status})`;
}

/** When a session started and what it held: its tasks, and its subagents when it started any. */
export function sessionStarted(session: SessionReceipt): string {
  const subagents = session.subagents > 0 ? `, ${countOf(session.subagents, 'subagent')}` : '';
  return `Started ${when(session.started_at_ms)}, ${countOf(session.tasks.length, 'task')}${subagents}.`;
}

/**
 * What a session took in total: its turns, output and cost when its usage is read, and its tool
 * calls (its input is in its overhead line); that its usage is not read when it is not, rather
 * than counts of nothing.
 */
export function sessionTotal(session: SessionReceipt): string {
  const calls = countOf(session.tool_calls, 'tool call');
  if (!session.is_metered) {
    return `Total: ${calls}. Cortex does not read this agent's transcripts, so its usage is not shown.`;
  }
  const { total } = session;
  if (total.turns === 0) return `Total: no usage recorded; ${calls}.`;
  const tokens = `${countOf(total.turns, 'turn')}, ${countOf(total.output, 'output token')}`;
  const priced =
    total.estimate === null ? tokens : `${tokens}, ${cost(total.estimate, total.turns)}`;
  return `Total: ${priced}; ${calls}.`;
}

/**
 * Cortex's overhead in a session: every token it injected that reached the agent, the primer's
 * among them, against the session's input when it is read.
 */
export function sessionOverhead(session: SessionReceipt): string {
  const primer =
    session.primer_tokens > 0 ? ` (primer: ${countOf(session.primer_tokens, 'token')})` : '';
  const against =
    session.is_metered && session.total.turns > 0
      ? `, against ${formatCount(session.total.cumulative_input)} input tokens in total`
      : '';
  return `${injected(session.injected_tokens)}${primer}${against}.`;
}

/** The compactions a session's total leaves out, or null when it leaves none out. */
export function compactionsSentence(session: SessionReceipt): string | null {
  if (session.compactions_left_out === 0) return null;
  return `Not in the total: ${countOf(session.compactions_left_out, 'compaction')}, which the transcript does not record.`;
}

/** What the panel says when no session started in the window. */
export function noSessionSentence(windowDays: number): string {
  return `No session started in the last ${countOf(windowDays, 'day')}. Receipts fill in as agents work in this repository with Cortex's hooks installed (cortex init sets them up).`;
}
