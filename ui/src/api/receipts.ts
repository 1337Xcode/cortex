// The receipts behind the map's receipts panel, the report `cortex receipts` prints: the price table
// costs were estimated from, the tasks held out from Cortex and the rest with their comparison once
// each arm has enough tasks that ended, how precise the work orders and warnings Cortex gave were,
// and each recent session's receipt, task by task. Every field is checked here, and so is the
// arithmetic that ties them together (a session's turns are its tasks' and the rest, a comparison
// exists only with enough tasks a side), so a body that breaks the contract is refused whole and
// nothing past this module checks a shape again.
import {
  API_VERSION,
  checkMaxLength,
  fail,
  readArray,
  readBoolean,
  readCappedString,
  readCount,
  readEnum,
  readEpochMs,
  readNullableBoolean,
  readRecord,
  readString,
  readVersion,
  runParse,
  type ApiResult,
  type JsonRecord,
} from './guard';
import { readAgent, readSessionId } from './live';
import { fetchChecked } from './request';

/** Where the receipts live, relative to the page. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const RECEIPTS_URL = 'api/receipts';

/** The most sessions the daemon sends: the newest of the window. */
export const MAX_RECEIPT_SESSIONS = 100;

/**
 * The most tasks one session may hold: a task is one prompt, and 2,000 prompts is more than a
 * person typing one a minute for a day and a half. A session past it is refused, not cut, so no
 * total ever stands beside only part of its tasks.
 */
export const MAX_TASKS_PER_SESSION = 2000;

/** The most cache breaks one session may list; a break needs a response of its own. */
export const MAX_CACHE_BREAKS_PER_SESSION = 5000;

/** The longest reason a price table could not be read that the page shows, in characters. */
export const MAX_PRICE_WHY_CHARS = 500;

/** A day as the price table writes it: `YYYY-MM-DD`. */
const DAY_TEXT = /^\d{4}-\d{2}-\d{2}$/;

/** A task id: a ULID, 26 characters of Crockford's base 32. */
const ULID_TEXT = /^[0-9A-HJKMNP-TV-Z]{26}$/;

/** What became of a session. */
export const SESSION_STATUSES = ['live', 'ended', 'abandoned', 'cleared'] as const;

/** How a task ended, once known. */
export const TASK_OUTCOMES = ['success', 'failure', 'reverted', 'abandoned', 'unknown'] as const;

/** Why a response has no estimated cost, in the order receipts list them. */
export const UNPRICED_REASONS = ['model', 'tier', 'speed', 'region', 'cache_split'] as const;

/** What most likely lost a conversation its prompt cache. */
export const BREAK_CAUSES = ['model_switch', 'compaction', 'idle_gap', 'hook_text'] as const;

/** One of `SESSION_STATUSES`. */
export type SessionStatus = (typeof SESSION_STATUSES)[number];
/** One of `TASK_OUTCOMES`. */
export type TaskOutcome = (typeof TASK_OUTCOMES)[number];
/** One of `UNPRICED_REASONS`. */
export type Unpriced = (typeof UNPRICED_REASONS)[number];
/** One of `BREAK_CAUSES`. */
export type BreakCause = (typeof BREAK_CAUSES)[number];

/** The price table costs were estimated from: the user's file, the one Cortex ships, or none. */
export type Prices =
  | { readonly state: 'read'; readonly updated: string }
  | { readonly state: 'shipped'; readonly updated: string }
  | { readonly state: 'missing'; readonly why: string };

/** One measure compared: each arm's value and the difference, with Cortex minus held out. */
export interface Compared {
  readonly treated: number;
  readonly held_out: number;
  readonly difference: number;
  /** The difference's 95% bootstrap interval. */
  readonly low: number;
  readonly high: number;
}

/** The holdout comparison, once each arm has enough tasks that ended with an outcome. */
export interface Comparison {
  /** Tasks with an outcome: with Cortex, then held out. */
  readonly tasks: readonly [number, number];
  readonly success_rate: Compared;
  /** Successful tasks whose usage was read: with Cortex, then held out. */
  readonly per_success_tasks: readonly [number, number];
  /** Medians per successful task (input tokens, uncached input, turns, tool calls), when enough. */
  readonly per_success: readonly [Compared, Compared, Compared, Compared] | null;
}

/** The tasks held out from Cortex and the rest. */
export interface Holdout {
  readonly held_out: number;
  readonly treated: number;
  readonly needed_per_arm: number;
  readonly comparison: Comparison | null;
}

/** Of what Cortex gave and labelled, how much was used. */
export interface Precision {
  readonly labelled: number;
  readonly used: number;
}

/** An estimated cost and the responses it leaves out. */
export interface Estimate {
  /**
   * In picodollars. The daemon writes a 128-bit count as a JSON number, so past 2^53 (about
   * $9,007) it arrives rounded to the nearest number JavaScript holds.
   */
  readonly picodollars: number;
  readonly cache_write_picodollars: number;
  readonly priced_turns: number;
  /** Responses with no price, by why; a reason with none is left out. */
  readonly unpriced_turns: Readonly<Partial<Record<Unpriced, number>>>;
}

/** Responses added up. */
export interface Tally {
  readonly turns: number;
  /** Every input token: read fresh, written to the cache and read from it. */
  readonly cumulative_input: number;
  readonly uncached_input: number;
  readonly cache_reads: number;
  readonly cache_writes: number;
  readonly output: number;
  /** Null when there is no price table. */
  readonly estimate: Estimate | null;
}

/** What one task took. */
export interface TaskReceipt {
  readonly task_id: string;
  /** Its number in the repository's order of tasks. */
  readonly seq: number;
  readonly started_at_ms: number;
  readonly ended_at_ms: number | null;
  readonly outcome: TaskOutcome | null;
  /** Whether Cortex held back from it, once decided. */
  readonly held_out: boolean | null;
  readonly tool_calls: number;
  readonly injected_tokens: number;
  readonly usage: Tally;
}

/** A response where a conversation lost its prompt cache. */
export interface CacheBreak {
  readonly session_id: string;
  readonly turn: number;
  readonly at_ms: number;
  readonly cause: BreakCause | null;
}

/** What one session took, task by task. */
export interface SessionReceipt {
  readonly session_id: string;
  readonly agent: string;
  /** Whether Cortex reads its agent's transcripts; when not, its tallies are empty and say nothing. */
  readonly is_metered: boolean;
  readonly started_at_ms: number;
  readonly ended_at_ms: number | null;
  readonly status: SessionStatus;
  readonly subagents: number;
  readonly tool_calls: number;
  /** In the order they started. */
  readonly tasks: readonly TaskReceipt[];
  readonly outside_tasks: Tally;
  readonly outside_tool_calls: number;
  readonly total: Tally;
  readonly compactions_left_out: number;
  readonly cache_breaks: readonly CacheBreak[];
  /** Tokens of the primers that reached the agent. */
  readonly primer_tokens: number;
  /** Every token Cortex added that reached the agent, the primers' among them. */
  readonly injected_tokens: number;
}

/** What the store holds. */
export interface Recorded {
  readonly holdout: Holdout;
  readonly precision: Precision;
  readonly warnings: Precision;
  /** By start, oldest first. */
  readonly sessions: readonly SessionReceipt[];
}

/** The whole body of `api/receipts`. */
export interface Receipts {
  readonly version: typeof API_VERSION;
  /** How many days back the sessions shown started. */
  readonly window_days: number;
  readonly prices: Prices;
  readonly recorded: Recorded;
}

/** The field as a finite number, or a `bad_shape` refusal. */
function readFinite(record: JsonRecord, key: string, where: string): number {
  const value = record[key];
  if (typeof value !== 'number' || !Number.isFinite(value)) {
    return fail('bad_shape', `${where}.${key}: expected a number`);
  }
  return value;
}

/**
 * The field as a whole number of picodollars from 0 up. Not a safe integer past 2^53: the daemon
 * writes a 128-bit count, which arrives rounded but whole.
 */
function readAmount(record: JsonRecord, key: string, where: string): number {
  const value = readFinite(record, key, where);
  if (!Number.isInteger(value) || value < 0) {
    return fail('bad_shape', `${where}.${key}: expected a whole amount from 0 up`);
  }
  return value;
}

/** The field as a pair of counts. */
function readCountPair(record: JsonRecord, key: string, where: string): [number, number] {
  const list = readArray(record, key, where);
  if (list.length !== 2) return fail('bad_shape', `${where}.${key}: expected two counts`);
  const pair = { first: list[0], second: list[1] };
  return [
    readCount(pair, 'first', `${where}.${key}`),
    readCount(pair, 'second', `${where}.${key}`),
  ];
}

/** The price table costs were estimated from, by its `state`. */
function parsePrices(value: unknown): Prices {
  const where = 'receipts.prices';
  const record = readRecord(value, where);
  const state = readEnum(record, 'state', ['read', 'shipped', 'missing'] as const, where);
  if (state === 'missing') {
    return { state, why: readCappedString(record, 'why', MAX_PRICE_WHY_CHARS, where) };
  }
  const updated = readString(record, 'updated', where);
  if (!DAY_TEXT.test(updated)) return fail('bad_shape', `${where}.updated: expected YYYY-MM-DD`);
  return { state, updated };
}

/** One measure compared, its difference and its interval checked against each other. */
function parseCompared(value: unknown, where: string): Compared {
  const record = readRecord(value, where);
  const compared = {
    treated: readFinite(record, 'treated', where),
    held_out: readFinite(record, 'held_out', where),
    difference: readFinite(record, 'difference', where),
    low: readFinite(record, 'low', where),
    high: readFinite(record, 'high', where),
  };
  // The daemon subtracts one double from the other; the same subtraction here gives the same bits.
  if (compared.difference !== compared.treated - compared.held_out) {
    return fail('bad_range', `${where}.difference: not with Cortex minus held out`);
  }
  if (compared.low > compared.high) return fail('bad_range', `${where}: low above high`);
  return compared;
}

/** Checks that a share and its difference stay within what shares can be. */
function checkShares(compared: Compared, where: string): void {
  const isShare = (value: number): boolean => value >= 0 && value <= 1;
  const isGap = (value: number): boolean => value >= -1 && value <= 1;
  if (!isShare(compared.treated) || !isShare(compared.held_out)) {
    fail('bad_range', `${where}: a rate outside 0 to 1`);
  }
  if (![compared.difference, compared.low, compared.high].every(isGap)) {
    fail('bad_range', `${where}: a difference of rates outside -1 to 1`);
  }
}

/** The four medians per successful task, or null when they are not given. */
function parsePerSuccess(record: JsonRecord, where: string): Comparison['per_success'] {
  const value = record['per_success'];
  if (value === null) return null;
  if (!Array.isArray(value) || value.length !== 4) {
    return fail('bad_shape', `${where}.per_success: expected four measures`);
  }
  const [a, b, c, d] = value.map((entry, index) => {
    const measure = parseCompared(entry, `${where}.per_success[${index}]`);
    if (measure.treated < 0 || measure.held_out < 0) {
      fail('bad_range', `${where}.per_success[${index}]: a negative median`);
    }
    return measure;
  });
  if (a === undefined || b === undefined || c === undefined || d === undefined) {
    return fail('bad_shape', `${where}.per_success: expected four measures`);
  }
  return [a, b, c, d];
}

/** The comparison, checked against the arms it compares and the tasks it needs. */
function parseComparison(
  value: unknown,
  arms: Pick<Holdout, 'held_out' | 'treated' | 'needed_per_arm'>,
): Comparison {
  const where = 'receipts.recorded.holdout.comparison';
  const record = readRecord(value, where);
  const success = parseCompared(record['success_rate'], `${where}.success_rate`);
  checkShares(success, `${where}.success_rate`);
  const comparison: Comparison = {
    tasks: readCountPair(record, 'tasks', where),
    success_rate: success,
    per_success_tasks: readCountPair(record, 'per_success_tasks', where),
    per_success: parsePerSuccess(record, where),
  };
  const needed = arms.needed_per_arm;
  const [treated, heldOut] = comparison.tasks;
  if (treated < needed || heldOut < needed) {
    return fail('bad_range', `${where}.tasks: fewer than ${needed} a side`);
  }
  if (treated > arms.treated || heldOut > arms.held_out) {
    return fail('bad_range', `${where}.tasks: more than the arms hold`);
  }
  const [withCortex, without] = comparison.per_success_tasks;
  if (withCortex > treated || without > heldOut) {
    return fail('bad_range', `${where}.per_success_tasks: more than the tasks compared`);
  }
  const isEnough = withCortex >= needed && without >= needed;
  if (isEnough !== (comparison.per_success !== null)) {
    return fail('bad_range', `${where}.per_success: given only with ${needed} a side`);
  }
  return comparison;
}

/** The arms and their comparison, which may come only once each arm has enough tasks. */
function parseHoldout(value: unknown): Holdout {
  const where = 'receipts.recorded.holdout';
  const record = readRecord(value, where);
  const arms = {
    held_out: readCount(record, 'held_out', where),
    treated: readCount(record, 'treated', where),
    needed_per_arm: readCount(record, 'needed_per_arm', where),
  };
  if (arms.needed_per_arm === 0) return fail('bad_range', `${where}.needed_per_arm: 0`);
  if (record['comparison'] === null) return { ...arms, comparison: null };
  if (arms.held_out < arms.needed_per_arm || arms.treated < arms.needed_per_arm) {
    return fail('bad_range', `${where}.comparison: given before each arm has enough tasks`);
  }
  return { ...arms, comparison: parseComparison(record['comparison'], arms) };
}

/** A precision: of what was labelled, how much was used, never more. */
function parsePrecision(value: unknown, where: string): Precision {
  const record = readRecord(value, where);
  const precision = {
    labelled: readCount(record, 'labelled', where),
    used: readCount(record, 'used', where),
  };
  if (precision.used > precision.labelled) {
    return fail('bad_range', `${where}.used: more than were labelled`);
  }
  return precision;
}

/** The responses with no price, by a reason the page knows, each counting at least one. */
function parseUnpriced(value: unknown, where: string): Estimate['unpriced_turns'] {
  const record = readRecord(value, where);
  const counts: Partial<Record<Unpriced, number>> = {};
  for (const key of Object.keys(record)) {
    const reason = readEnum({ key }, 'key', UNPRICED_REASONS, where);
    const count = readCount(record, key, where);
    if (count === 0) return fail('bad_range', `${where}.${key}: a reason with no response`);
    counts[reason] = count;
  }
  return counts;
}

/** An estimate of `turns` responses: each priced or counted with why it has no price. */
function parseEstimate(value: unknown, turns: number, where: string): Estimate {
  const record = readRecord(value, where);
  const estimate: Estimate = {
    picodollars: readAmount(record, 'picodollars', where),
    cache_write_picodollars: readAmount(record, 'cache_write_picodollars', where),
    priced_turns: readCount(record, 'priced_turns', where),
    unpriced_turns: parseUnpriced(record['unpriced_turns'], `${where}.unpriced_turns`),
  };
  if (estimate.cache_write_picodollars > estimate.picodollars) {
    return fail('bad_range', `${where}.cache_write_picodollars: more than the whole cost`);
  }
  const unpriced = Object.values(estimate.unpriced_turns).reduce((sum, count) => sum + count, 0);
  if (estimate.priced_turns + unpriced !== turns) {
    return fail('bad_range', `${where}: priced and unpriced responses are not the tally's`);
  }
  return estimate;
}

/** A tally; `isPriced` says whether the report has a price table, so whether it has an estimate. */
function parseTally(value: unknown, isPriced: boolean, where: string): Tally {
  const record = readRecord(value, where);
  const turns = readCount(record, 'turns', where);
  const tally: Tally = {
    turns,
    cumulative_input: readCount(record, 'cumulative_input', where),
    uncached_input: readCount(record, 'uncached_input', where),
    cache_reads: readCount(record, 'cache_reads', where),
    cache_writes: readCount(record, 'cache_writes', where),
    output: readCount(record, 'output', where),
    estimate:
      record['estimate'] === null
        ? null
        : parseEstimate(record['estimate'], turns, `${where}.estimate`),
  };
  if (tally.uncached_input + tally.cache_reads + tally.cache_writes !== tally.cumulative_input) {
    return fail('bad_range', `${where}.cumulative_input: not its three parts added up`);
  }
  if ((tally.estimate !== null) !== isPriced) {
    return fail('bad_range', `${where}.estimate: ${isPriced ? 'missing' : 'given with no prices'}`);
  }
  return tally;
}

/** One task's receipt; `isPriced` says whether its usage carries an estimate. */
function parseTask(value: unknown, isPriced: boolean, where: string): TaskReceipt {
  const record = readRecord(value, where);
  const id = readString(record, 'task_id', where);
  if (!ULID_TEXT.test(id)) return fail('bad_shape', `${where}.task_id: expected a ULID`);
  return {
    task_id: id,
    seq: readCount(record, 'seq', where),
    started_at_ms: readEpochMs(record, 'started_at_ms', where),
    ended_at_ms: record['ended_at_ms'] === null ? null : readEpochMs(record, 'ended_at_ms', where),
    outcome: record['outcome'] === null ? null : readEnum(record, 'outcome', TASK_OUTCOMES, where),
    held_out: readNullableBoolean(record, 'held_out', where),
    tool_calls: readCount(record, 'tool_calls', where),
    injected_tokens: readCount(record, 'injected_tokens', where),
    usage: parseTally(record['usage'], isPriced, `${where}.usage`),
  };
}

/** One response where a conversation lost its prompt cache. */
function parseBreak(value: unknown, where: string): CacheBreak {
  const record = readRecord(value, where);
  return {
    session_id: readSessionId(record, where, 'session_id'),
    turn: readCount(record, 'turn', where),
    at_ms: readEpochMs(record, 'at_ms', where),
    cause: record['cause'] === null ? null : readEnum(record, 'cause', BREAK_CAUSES, where),
  };
}

/** Refuses `list` unless its `startOf` values never go down. */
function checkByStart<T>(list: readonly T[], startOf: (entry: T) => number, where: string): void {
  for (let i = 1; i < list.length; i += 1) {
    const [before, entry] = [list[i - 1], list[i]];
    if (before !== undefined && entry !== undefined && startOf(entry) < startOf(before)) {
      fail('bad_order', `${where}[${i}]: started before the one above it`);
    }
  }
}

/** The tasks of a session, checked for count and order. */
function parseTasks(record: JsonRecord, isPriced: boolean, where: string): TaskReceipt[] {
  const list = readArray(record, 'tasks', where);
  checkMaxLength(list, MAX_TASKS_PER_SESSION, `${where}.tasks`);
  const tasks = list.map((entry, i) => parseTask(entry, isPriced, `${where}.tasks[${i}]`));
  checkByStart(tasks, (task) => task.started_at_ms, `${where}.tasks`);
  return tasks;
}

/** Refuses a session whose totals are not its parts added up. */
function checkSessionSums(session: SessionReceipt, where: string): void {
  const sum = (count: (task: TaskReceipt) => number): number =>
    session.tasks.reduce((total, task) => total + count(task), 0);
  if (sum((task) => task.usage.turns) + session.outside_tasks.turns !== session.total.turns) {
    fail('bad_range', `${where}.total.turns: not its tasks' and the rest added up`);
  }
  if (sum((task) => task.tool_calls) + session.outside_tool_calls !== session.tool_calls) {
    fail('bad_range', `${where}.tool_calls: not its tasks' and the rest added up`);
  }
  if (sum((task) => task.injected_tokens) + session.primer_tokens > session.injected_tokens) {
    fail('bad_range', `${where}.injected_tokens: fewer than its primers' and its tasks'`);
  }
  if (!session.is_metered && (session.total.turns > 0 || session.cache_breaks.length > 0)) {
    fail('bad_range', `${where}: usage of a session whose transcripts are not read`);
  }
}

/** The session at `index`, its totals checked against its tasks. */
function parseSession(value: unknown, isPriced: boolean, index: number): SessionReceipt {
  const where = `receipts.recorded.sessions[${index}]`;
  const record = readRecord(value, where);
  const breaks = readArray(record, 'cache_breaks', where);
  checkMaxLength(breaks, MAX_CACHE_BREAKS_PER_SESSION, `${where}.cache_breaks`);
  const session: SessionReceipt = {
    session_id: readSessionId(record, where, 'session_id'),
    agent: readAgent(record, where),
    is_metered: readBoolean(record, 'is_metered', where),
    started_at_ms: readEpochMs(record, 'started_at_ms', where),
    ended_at_ms: record['ended_at_ms'] === null ? null : readEpochMs(record, 'ended_at_ms', where),
    status: readEnum(record, 'status', SESSION_STATUSES, where),
    subagents: readCount(record, 'subagents', where),
    tool_calls: readCount(record, 'tool_calls', where),
    tasks: parseTasks(record, isPriced, where),
    outside_tasks: parseTally(record['outside_tasks'], isPriced, `${where}.outside_tasks`),
    outside_tool_calls: readCount(record, 'outside_tool_calls', where),
    total: parseTally(record['total'], isPriced, `${where}.total`),
    compactions_left_out: readCount(record, 'compactions_left_out', where),
    cache_breaks: breaks.map((entry, i) => parseBreak(entry, `${where}.cache_breaks[${i}]`)),
    primer_tokens: readCount(record, 'primer_tokens', where),
    injected_tokens: readCount(record, 'injected_tokens', where),
  };
  checkSessionSums(session, where);
  return session;
}

/** What the store holds: the holdout, the precisions and the sessions, oldest first. */
function parseRecorded(value: unknown, isPriced: boolean): Recorded {
  const where = 'receipts.recorded';
  const record = readRecord(value, where);
  const list = readArray(record, 'sessions', where);
  checkMaxLength(list, MAX_RECEIPT_SESSIONS, `${where}.sessions`);
  const sessions = list.map((entry, index) => parseSession(entry, isPriced, index));
  checkByStart(sessions, (session) => session.started_at_ms, `${where}.sessions`);
  return {
    holdout: parseHoldout(record['holdout']),
    precision: parsePrecision(record['precision'], `${where}.precision`),
    warnings: parsePrecision(record['warnings'], `${where}.warnings`),
    sessions,
  };
}

/** The whole body; the price state decides whether every tally carries an estimate. */
function parseReceiptsRecord(value: unknown): Receipts {
  const where = 'receipts';
  const record = readRecord(value, where);
  readVersion(record, where);
  const windowDays = readCount(record, 'window_days', where);
  if (windowDays === 0) return fail('bad_range', `${where}.window_days: 0`);
  const prices = parsePrices(record['prices']);
  return {
    version: API_VERSION,
    window_days: windowDays,
    prices,
    recorded: parseRecorded(record['recorded'], prices.state !== 'missing'),
  };
}

/** Checks parsed JSON against the receipts contract. */
export function parseReceipts(value: unknown): ApiResult<Receipts> {
  return runParse(parseReceiptsRecord, value);
}

/** Fetches `api/receipts` from the daemon that served the page and checks it. */
export function fetchReceipts(signal: AbortSignal): Promise<ApiResult<Receipts>> {
  return fetchChecked(RECEIPTS_URL, signal, parseReceipts);
}
