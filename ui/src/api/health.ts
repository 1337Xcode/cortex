// The daemon's health as the map reads it: its Cortex version, whether it holds the map's files,
// whether it is still reading the repository for the first time, and how the daemon is doing (what
// `cortex status` shows: its uptime, each hook's handling times, its newest errors, dropped events,
// the co-change miner's state, the graph's size, upkeep's last run and the history worker). The
// empty map asks for it, so it can say "still reading" only while that is true; the health section
// of the overview shows the rest.
import {
  checkMaxLength,
  fail,
  readArray,
  readBoolean,
  readCappedString,
  readCount,
  readEnum,
  readEpochMs,
  readRecord,
  runParse,
  type ApiError,
  type ApiResult,
  type JsonRecord,
} from './guard';
import { HISTORY_READERS, type HistoryReader } from './file';
import { fetchChecked } from './request';

/** Where the health lives, relative to the page. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const HEALTH_URL = 'api/health';

/** The longest version text the page accepts, in characters. */
const MAX_VERSION_CHARS = 64;

/** The longest hook name the page accepts, in characters. */
export const MAX_HOOK_NAME_CHARS = 64;

/**
 * The longest error message or miner detail the page accepts, in characters: the daemon cuts both
 * to this length, on one line.
 */
export const MAX_MESSAGE_CHARS = 1000;

/** The most errors the daemon names: its newest. */
export const MAX_ERRORS = 10;

/** The most hooks a report lists; the daemon knows nine, so more is not a report it sends. */
export const MAX_HOOKS = 64;

/** Whether the first reading of the repository is still under way. */
export const INDEX_STATES = ['reading', 'ready'] as const;

/** One of `INDEX_STATES`. */
export type IndexState = (typeof INDEX_STATES)[number];

/** Where the co-change miner is; `other` is a state a newer daemon names that this page does not know. */
export const GIT_STATES = [
  'waiting',
  'no_repository',
  'mining',
  'ready',
  'failed',
  'other',
] as const;

/** One of `GIT_STATES`. */
export type GitState = (typeof GIT_STATES)[number];

/**
 * How long the daemon took to handle one hook's latest requests, in microseconds: its own handling,
 * from the request's first byte to its answer, not the hook process's whole run.
 */
export interface HookTimes {
  /** The hook's name, such as `pre_tool`; a newer daemon may name one this page does not know. */
  readonly hook: string;
  /** How many of the hook's latest requests the percentiles cover. */
  readonly requests: number;
  readonly p50_us: number;
  readonly p95_us: number;
  readonly p99_us: number;
}

/** An error the daemon met, in its words, without file contents, prompt text or tool output. */
export interface DaemonError {
  /** When it last happened, in milliseconds since the epoch. */
  readonly at_ms: number;
  /** How many times it happened while the daemon kept it. */
  readonly count: number;
  readonly message: string;
}

/** How the daemon is doing, from its status report at the moment of the request. */
export interface DaemonHealth {
  readonly uptime_ms: number;
  readonly hooks: readonly HookTimes[];
  /** Its newest errors, newest first, at most `MAX_ERRORS`. */
  readonly errors: readonly DaemonError[];
  /** Hook events dropped because too many waited, since it started. */
  readonly events_dropped: number;
  /** Batches of events the store could not record, since it started. */
  readonly failed_batches: number;
  readonly git: GitState;
  readonly git_detail: string | null;
  /** The graph retrieval spreads over, as last built: null for both before its first build. */
  readonly graph: { readonly nodes: number; readonly links: number } | null;
  /** When upkeep last ran, or null before its first run. */
  readonly upkeep_at_ms: number | null;
  readonly history: HistoryReader;
}

/**
 * The daemon's report as the page could take it: read, absent (the daemon could not say, as while
 * it stops), or unreadable, with why. A report the page cannot read is refused on its own, so the
 * rest of the answer (whether the daemon is still reading the repository) still counts.
 */
export type DaemonReport =
  | { readonly status: 'read'; readonly health: DaemonHealth }
  | { readonly status: 'absent' }
  | { readonly status: 'unreadable'; readonly error: ApiError };

/** The daemon's health. */
export interface Health {
  /** The daemon's Cortex version, as it names it. */
  readonly version: string;
  /** Whether this build holds the map's files. */
  readonly has_assets: boolean;
  readonly index: IndexState;
  /** How the daemon is doing. */
  readonly daemon: DaemonReport;
}

/** The field as one line of text of at most `MAX_MESSAGE_CHARS` characters, or null. */
function readNullableMessage(record: JsonRecord, key: string, where: string): string | null {
  return record[key] === null ? null : readCappedString(record, key, MAX_MESSAGE_CHARS, where);
}

function parseHook(value: unknown, index: number): HookTimes {
  const where = `health.daemon.hooks[${index}]`;
  const record = readRecord(value, where);
  const hook = readCappedString(record, 'hook', MAX_HOOK_NAME_CHARS, where);
  if (hook.length === 0) return fail('bad_shape', `${where}.hook: expected a name`);
  const times = {
    p50_us: readCount(record, 'p50_us', where),
    p95_us: readCount(record, 'p95_us', where),
    p99_us: readCount(record, 'p99_us', where),
  };
  if (times.p50_us > times.p95_us || times.p95_us > times.p99_us) {
    return fail('bad_order', `${where}: percentiles must not fall`);
  }
  return { hook, requests: readCount(record, 'requests', where), ...times };
}

function parseError(value: unknown, index: number): DaemonError {
  const where = `health.daemon.errors[${index}]`;
  const record = readRecord(value, where);
  const count = readCount(record, 'count', where);
  if (count < 1) return fail('bad_range', `${where}.count: an error happened at least once`);
  return {
    at_ms: readEpochMs(record, 'at_ms', where),
    count,
    message: readCappedString(record, 'message', MAX_MESSAGE_CHARS, where),
  };
}

/** The graph's size: both counts, or neither before its first build. */
function readGraphSize(record: JsonRecord, where: string): DaemonHealth['graph'] {
  const nodes = record['graph_nodes'] === null ? null : readCount(record, 'graph_nodes', where);
  const links = record['graph_links'] === null ? null : readCount(record, 'graph_links', where);
  if (nodes === null && links === null) return null;
  if (nodes === null || links === null) {
    return fail('bad_shape', `${where}: the graph's nodes and links come together`);
  }
  return { nodes, links };
}

function parseDaemon(value: unknown): DaemonHealth {
  const where = 'health.daemon';
  const record = readRecord(value, where);
  const hookList = readArray(record, 'hooks', where);
  checkMaxLength(hookList, MAX_HOOKS, `${where}.hooks`);
  const errorList = readArray(record, 'errors', where);
  checkMaxLength(errorList, MAX_ERRORS, `${where}.errors`);
  return {
    uptime_ms: readCount(record, 'uptime_ms', where),
    hooks: hookList.map(parseHook),
    // Listed in the daemon's order, newest first; their times are not checked against each other,
    // as a clock set back would make a true list look out of order.
    errors: errorList.map(parseError),
    events_dropped: readCount(record, 'events_dropped', where),
    failed_batches: readCount(record, 'failed_batches', where),
    git: readEnum(record, 'git', GIT_STATES, where),
    git_detail: readNullableMessage(record, 'git_detail', where),
    graph: readGraphSize(record, where),
    upkeep_at_ms:
      record['upkeep_at_ms'] === null ? null : readEpochMs(record, 'upkeep_at_ms', where),
    history: readEnum(record, 'history', HISTORY_READERS, where),
  };
}

/** The daemon's report from `value`, refused on its own when the page cannot read it. */
function readDaemonReport(value: unknown): DaemonReport {
  if (value === null) return { status: 'absent' };
  const report = runParse(parseDaemon, value);
  return report.ok
    ? { status: 'read', health: report.value }
    : { status: 'unreadable', error: report.error };
}

function parseHealthRecord(value: unknown): Health {
  const where = 'health';
  const record = readRecord(value, where);
  const version = readCappedString(record, 'version', MAX_VERSION_CHARS, where);
  if (version.length === 0) return fail('bad_shape', `${where}.version: expected a version`);
  if (!('daemon' in record))
    return fail('bad_shape', `${where}.daemon: expected an object or null`);
  return {
    version,
    has_assets: readBoolean(record, 'has_assets', where),
    index: readEnum(record, 'index', INDEX_STATES, where),
    daemon: readDaemonReport(record['daemon']),
  };
}

/** Checks parsed JSON against the health contract. */
export function parseHealth(value: unknown): ApiResult<Health> {
  return runParse(parseHealthRecord, value);
}

/** Fetches `api/health` from the daemon that served the page and checks it. */
export function fetchHealth(signal: AbortSignal): Promise<ApiResult<Health>> {
  return fetchChecked(HEALTH_URL, signal, parseHealth);
}
