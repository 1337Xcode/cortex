// The daemon's health as the map reads it: its Cortex version, whether it holds the map's files,
// and whether it is still reading the repository for the first time. The empty map asks for it, so
// it can say "still reading" only while that is true, and show the plain empty state once it is not.
import {
  fail,
  readBoolean,
  readCappedString,
  readEnum,
  readRecord,
  runParse,
  type ApiResult,
} from './guard';
import { fetchChecked } from './request';

/** Where the health lives, relative to the page. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const HEALTH_URL = 'api/health';

/** The longest version text the page accepts, in characters. */
const MAX_VERSION_CHARS = 64;

/** Whether the first reading of the repository is still under way. */
export const INDEX_STATES = ['reading', 'ready'] as const;

/** One of `INDEX_STATES`. */
export type IndexState = (typeof INDEX_STATES)[number];

/** The daemon's health. */
export interface Health {
  /** The daemon's Cortex version, as it names it. */
  readonly version: string;
  /** Whether this build holds the map's files. */
  readonly has_assets: boolean;
  readonly index: IndexState;
}

function parseHealthRecord(value: unknown): Health {
  const where = 'health';
  const record = readRecord(value, where);
  const version = readCappedString(record, 'version', MAX_VERSION_CHARS, where);
  if (version.length === 0) return fail('bad_shape', `${where}.version: expected a version`);
  return {
    version,
    has_assets: readBoolean(record, 'has_assets', where),
    index: readEnum(record, 'index', INDEX_STATES, where),
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
