// The summary behind the first screen's strip of sentences: counts of files, folders and explored
// files, whether anything has been learned, the strongest co-change pair and the busiest folder.
// Parsed by hand at the edge like every other response.
import {
  API_VERSION,
  fail,
  readBoolean,
  readCount,
  readPath,
  readRecord,
  readVersion,
  runParse,
  type ApiResult,
  type JsonRecord,
} from './guard';
import { fetchChecked } from './request';

/** Where the summary lives, relative to the page. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const SUMMARY_URL = 'api/summary';

/** The co-change link with the most commits. */
export interface StrongestPair {
  readonly from: number;
  readonly to: number;
  readonly commits: number;
}

/** The folder whose files took the most tokens in the window. */
export interface BusiestFolder {
  readonly path: string;
  readonly tokens: number;
}

/** The first screen's numbers, all measured from the store. */
export interface MapSummary {
  readonly version: typeof API_VERSION;
  readonly files: number;
  readonly folders: number;
  /** Files agents have read at least once. */
  readonly explored_files: number;
  /** Whether any link, fact, read or token is stored; false for a cold repository. */
  readonly learned: boolean;
  readonly strongest_pair: StrongestPair | null;
  readonly busiest_folder: BusiestFolder | null;
  /** The window, in days, the busiest folder is measured over. */
  readonly window_days: number;
}

function readPair(record: JsonRecord): StrongestPair | null {
  const value = record['strongest_pair'];
  if (value === null) return null;
  const where = 'summary.strongest_pair';
  const pair = readRecord(value, where);
  const from = readCount(pair, 'from', where);
  const to = readCount(pair, 'to', where);
  if (from >= to) return fail('bad_range', `${where}: from must be below to`);
  return { from, to, commits: readCount(pair, 'commits', where) };
}

function readFolder(record: JsonRecord): BusiestFolder | null {
  const value = record['busiest_folder'];
  if (value === null) return null;
  const where = 'summary.busiest_folder';
  const folder = readRecord(value, where);
  return { path: readPath(folder, 'path', where), tokens: readCount(folder, 'tokens', where) };
}

function parseSummaryRecord(value: unknown): MapSummary {
  const where = 'summary';
  const record = readRecord(value, where);
  readVersion(record, where);
  const summary: MapSummary = {
    version: API_VERSION,
    files: readCount(record, 'files', where),
    folders: readCount(record, 'folders', where),
    explored_files: readCount(record, 'explored_files', where),
    learned: readBoolean(record, 'learned', where),
    strongest_pair: readPair(record),
    busiest_folder: readFolder(record),
    window_days: readCount(record, 'window_days', where),
  };
  if (summary.explored_files > summary.files) {
    return fail('bad_range', `${where}.explored_files: more than the ${summary.files} files`);
  }
  const hasLearned = summary.explored_files > 0 || summary.strongest_pair !== null;
  if (hasLearned && !summary.learned) {
    return fail('bad_range', `${where}.learned: false, but reads or links are stored`);
  }
  return summary;
}

/** Checks parsed JSON against the summary contract. */
export function parseSummary(value: unknown): ApiResult<MapSummary> {
  return runParse(parseSummaryRecord, value);
}

/** Fetches `api/summary` from the daemon that served the page and checks it. */
export function fetchSummary(signal: AbortSignal): Promise<ApiResult<MapSummary>> {
  return fetchChecked(SUMMARY_URL, signal, parseSummary);
}
