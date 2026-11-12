// One file's detail for the side panel: the facts anchored to it, the stored history of its
// blamed commits, and the times Cortex named it to an agent with whether the agent then used it.
// Links come from the graph snapshot and are not repeated here.
import {
  API_VERSION,
  checkMaxLength,
  codePointLength,
  fail,
  readArray,
  readCappedString,
  readCount,
  readEpochMs,
  readEpochSeconds,
  readEnum,
  readNullableBoolean,
  readPath,
  readRecord,
  readString,
  readVersion,
  runParse,
  type ApiResult,
  type JsonRecord,
} from './guard';
import { fetchChecked } from './request';

/** The longest fact text the daemon stores, in characters. */
export const MAX_FACT_CHARS = 240;

/** The longest commit subject the page accepts, in characters. */
export const MAX_SUBJECT_CHARS = 200;

/** The longest commit references (issue and pull request numbers) the page accepts, in characters. */
export const MAX_REFS_CHARS = 80;

/** The most history entries the daemon returns. */
export const MAX_HISTORY = 10;

/** The most "told" entries the daemon returns. */
export const MAX_TOLD = 20;

/** A fact's state: confirmed, waiting for confirmation, or no longer matching the code. */
export const FACT_STATUSES = ['active', 'candidate', 'suspect'] as const;

/** One of `FACT_STATUSES`. */
export type FactStatus = (typeof FACT_STATUSES)[number];

/** How Cortex named the file to an agent. */
export const TOLD_KINDS = ['work_order', 'warning'] as const;

/** One of `TOLD_KINDS`. */
export type ToldKind = (typeof TOLD_KINDS)[number];

/** A fact anchored to the file. */
export interface FileFact {
  readonly id: number;
  readonly kind: string;
  readonly status: FactStatus;
  readonly text: string;
}

/** A commit the stored blames of the file name. */
export interface FileHistoryEntry {
  readonly subject: string;
  readonly refs: string;
  /** Seconds since the epoch. */
  readonly date_s: number;
}

/** One time Cortex named the file to an agent. */
export interface ToldEntry {
  readonly kind: ToldKind;
  readonly at_ms: number;
  /** Whether the agent then read or edited the file in that task; null when not yet known. */
  readonly used: boolean | null;
}

/** The side panel's data for one file. */
export interface FileDetail {
  readonly version: typeof API_VERSION;
  readonly id: number;
  readonly path: string;
  readonly facts: readonly FileFact[];
  readonly history: readonly FileHistoryEntry[];
  readonly told: readonly ToldEntry[];
}

/** The relative URL of one file's detail. */
export function fileUrl(id: number): string {
  // INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
  return `api/file/${id}`;
}

function parseFact(value: unknown, index: number): FileFact {
  const where = `facts[${index}]`;
  const record = readRecord(value, where);
  const kind = readString(record, 'kind', where);
  if (kind.length === 0) return fail('bad_shape', `${where}.kind: expected a kind`);
  const text = readString(record, 'text', where);
  if (codePointLength(text) > MAX_FACT_CHARS) {
    return fail('bad_range', `${where}.text: longer than ${MAX_FACT_CHARS} characters`);
  }
  const status = readEnum(record, 'status', FACT_STATUSES, where);
  return { id: readCount(record, 'id', where), kind, status, text };
}

function parseHistoryEntry(value: unknown, index: number): FileHistoryEntry {
  const where = `history[${index}]`;
  const record = readRecord(value, where);
  return {
    subject: readCappedString(record, 'subject', MAX_SUBJECT_CHARS, where),
    refs: readCappedString(record, 'refs', MAX_REFS_CHARS, where),
    date_s: readEpochSeconds(record, 'date_s', where),
  };
}

function parseTold(value: unknown, index: number): ToldEntry {
  const where = `told[${index}]`;
  const record = readRecord(value, where);
  return {
    kind: readEnum(record, 'kind', TOLD_KINDS, where),
    at_ms: readEpochMs(record, 'at_ms', where),
    used: readNullableBoolean(record, 'used', where),
  };
}

/** Parses each entry and refuses the list unless `isInOrder` holds for every neighbouring pair. */
function parseOrdered<T>(
  record: JsonRecord,
  key: string,
  parse: (value: unknown, index: number) => T,
  isInOrder: (previous: T, next: T) => boolean,
  rule: string,
): T[] {
  const list = readArray(record, key, 'file').map(parse);
  for (let i = 1; i < list.length; i += 1) {
    const previous = list[i - 1];
    const next = list[i];
    if (previous !== undefined && next !== undefined && !isInOrder(previous, next)) {
      return fail('bad_order', `${key}[${i}]: ${rule}`);
    }
  }
  return list;
}

function detailParser(askedId: number): (value: unknown) => FileDetail {
  return (value) => {
    const record = readRecord(value, 'file');
    readVersion(record, 'file');
    const id = readCount(record, 'id', 'file');
    if (id !== askedId) return fail('bad_range', `file.id: asked for ${askedId}, got ${id}`);
    const facts = parseOrdered(record, 'facts', parseFact, (a, b) => a.id < b.id, 'facts by id');
    const history = parseOrdered(
      record,
      'history',
      parseHistoryEntry,
      (a, b) => a.date_s >= b.date_s,
      'history newest first',
    );
    const told = parseOrdered(
      record,
      'told',
      parseTold,
      (a, b) => a.at_ms >= b.at_ms,
      'newest first',
    );
    checkMaxLength(history, MAX_HISTORY, 'file.history');
    checkMaxLength(told, MAX_TOLD, 'file.told');
    return {
      version: API_VERSION,
      id,
      path: readPath(record, 'path', 'file'),
      facts,
      history,
      told,
    };
  };
}

/** Checks parsed JSON against the file contract for the id `askedId`. */
export function parseFileDetail(value: unknown, askedId: number): ApiResult<FileDetail> {
  return runParse(detailParser(askedId), value);
}

/** Fetches `api/file/<id>` and checks it; an id that is not a file comes back as `not_found`. */
export function fetchFileDetail(id: number, signal: AbortSignal): Promise<ApiResult<FileDetail>> {
  return fetchChecked(fileUrl(id), signal, (value) => parseFileDetail(value, id));
}
