// The warnings Cortex gave agents after their edits, as `api/warnings` lists them, newest first,
// open and resolved: each the kind of warning (a file that usually changes with the one edited, its
// tests, or a file that calls a symbol whose signature changed), the file the edit was about and
// the file the warning named (each by its id and its path), how strong its evidence was, and what
// the end of its task found.
// Notes of where edited code came from name no other file and are not in it.
import {
  API_VERSION,
  checkMaxLength,
  readArray,
  readEnum,
  readEpochMs,
  readNullableCount,
  readNullablePath,
  readRecord,
  readUnit,
  readVersion,
  runParse,
  type ApiResult,
} from './guard';
import { fetchChecked } from './request';

/** Where the warnings live, relative to the page. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const WARNINGS_URL = 'api/warnings';

/** The most warnings the daemon reads: its newest. */
export const MAX_WARNINGS = 100;

/** What a warning was about, as the store names its kinds. */
export const WARNING_KINDS = ['cochange', 'test', 'call_site'] as const;

/** One of `WARNING_KINDS`. */
export type WarningKind = (typeof WARNING_KINDS)[number];

/**
 * What became of a warning: open until its task ends, then followed (the named file was read or
 * edited after it) or not followed.
 */
export const WARNING_STATES = ['open', 'followed', 'not_followed'] as const;

/** One of `WARNING_STATES`. */
export type WarningState = (typeof WARNING_STATES)[number];

/** One file a warning named. */
export interface GivenWarning {
  /** When, in milliseconds since the epoch. */
  readonly at_ms: number;
  readonly kind: WarningKind;
  /**
   * The file the edit was about, by its id in the daemon's `api/graph` when it answered; null when
   * the map no longer lists it. Ids move when files are added, so check it against `source_path`.
   */
  readonly source: number | null;
  /** The file the warning named, by its id; null when the map no longer lists it. */
  readonly target: number | null;
  /** The edited file's path as the store holds it; null when the store no longer holds it. */
  readonly source_path: string | null;
  /** The named file's path, likewise. */
  readonly target_path: string | null;
  /** How strong its evidence was, from 0 to 1; what it measures depends on the kind. */
  readonly strength: number;
  readonly state: WarningState;
}

/** The warnings, newest first. */
export interface Warnings {
  readonly version: typeof API_VERSION;
  readonly warnings: readonly GivenWarning[];
}

function parseWarning(value: unknown, index: number): GivenWarning {
  const where = `warnings[${index}]`;
  const record = readRecord(value, where);
  return {
    at_ms: readEpochMs(record, 'at_ms', where),
    kind: readEnum(record, 'kind', WARNING_KINDS, where),
    source: readNullableCount(record, 'source', where),
    target: readNullableCount(record, 'target', where),
    source_path: readNullablePath(record, 'source_path', where),
    target_path: readNullablePath(record, 'target_path', where),
    strength: readUnit(record, 'strength', where),
    state: readEnum(record, 'state', WARNING_STATES, where),
  };
}

function parseWarningsRecord(value: unknown): Warnings {
  const record = readRecord(value, 'warnings');
  readVersion(record, 'warnings');
  const list = readArray(record, 'warnings', 'warnings');
  checkMaxLength(list, MAX_WARNINGS, 'warnings.warnings');
  // In the store's order, newest first; their times are not checked against each other.
  return { version: API_VERSION, warnings: list.map(parseWarning) };
}

/** Checks parsed JSON against the warnings contract. */
export function parseWarnings(value: unknown): ApiResult<Warnings> {
  return runParse(parseWarningsRecord, value);
}

/** Fetches `api/warnings` from the daemon that served the page and checks it. */
export function fetchWarnings(signal: AbortSignal): Promise<ApiResult<Warnings>> {
  return fetchChecked(WARNINGS_URL, signal, parseWarnings);
}
