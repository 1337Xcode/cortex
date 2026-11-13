// One file's detail for the side panel: the facts anchored to it, the stored history of its
// blamed commits (the newest ten, how many there are, and whether the history worker runs), the
// symbols it defines with the spans the latest work orders selected, the times Cortex named it to
// an agent with whether the agent then used it, and the link that opens it in the person's editor.
// Links come from the graph snapshot and are not repeated here.
import {
  API_VERSION,
  checkMaxLength,
  codePointLength,
  fail,
  readArray,
  readBoolean,
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

/**
 * The longest commit references (a pull request number and issue references) the page accepts, in
 * characters: the daemon writes a pull request as `#` and up to 20 digits (the most a 64-bit count
 * has), then up to 3 issue references of up to 100 characters each, each after a space, so
 * 1 + 20 + 3 x (1 + 100) = 324.
 */
export const MAX_REFS_CHARS = 1 + 20 + 3 * (1 + 100);

/** The most history entries the daemon returns. */
export const MAX_HISTORY = 10;

/** The most "told" entries the daemon returns. */
export const MAX_TOLD = 20;

/**
 * The longest editor link the page follows, in characters: the daemon's template is at most 200
 * characters, and it writes the file's absolute path into it, at most 4,096 bytes on the platforms
 * Cortex runs on, each byte as up to 3 characters (`%XX`), so 200 + 3 x 4,096 = 12,488. A template
 * that names the path more than once can pass it; such a link is dropped, never the panel.
 */
export const MAX_EDITOR_URL_CHARS = 200 + 3 * 4096;

/** The most symbols the daemon lists for a file: the first in it, in line order. */
export const MAX_SYMBOLS = 500;

/** Whether the history worker reads blames: it runs, it is off, or git is refused to it. */
export const HISTORY_READERS = ['on', 'off', 'refused'] as const;

/** One of `HISTORY_READERS`. */
export type HistoryReader = (typeof HISTORY_READERS)[number];

/** What a symbol is, as the index names it. */
export const SYMBOL_KINDS = [
  'function',
  'method',
  'class',
  'interface',
  'type',
  'constant',
  'module',
  'test',
] as const;

/** One of `SYMBOL_KINDS`. */
export type SymbolKind = (typeof SYMBOL_KINDS)[number];

/** A symbol the file defines, with how many of the latest work orders selected lines of it. */
export interface FileSymbol {
  readonly name: string;
  readonly kind: SymbolKind;
  /** The 1-based line where it starts, as of the index's last read of the file. */
  readonly start_line: number;
  readonly end_line: number;
  /** Whether another symbol of the file holds it. */
  readonly is_nested: boolean;
  /** How many of the work orders read (the latest `MAX_TOLD` that listed the file) selected it. */
  readonly selected: number;
}

/** A span of the file that work orders selected, by its lines. */
export interface SelectedSpan {
  readonly start_line: number;
  readonly end_line: number;
  /** How many of the work orders read selected it, from 1. */
  readonly orders: number;
}

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
  /** The newest commits behind the file's current lines, newest first, at most `MAX_HISTORY`. */
  readonly history: readonly FileHistoryEntry[];
  /** How many such commits the panel could name; `history` is the newest of them. */
  readonly history_total: number;
  readonly history_reader: HistoryReader;
  /** The first `MAX_SYMBOLS` symbols the file defines, in line order. */
  readonly symbols: readonly FileSymbol[];
  /** How many symbols the file defines. */
  readonly symbols_total: number;
  /** The spans the latest work orders that listed the file selected, by their lines. */
  readonly selected: readonly SelectedSpan[];
  readonly told: readonly ToldEntry[];
  /** The daemon's link to open the file in an editor (`vscode://file/...`), or null when turned off. */
  readonly editor_url: string | null;
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

/** The field as a line number, from 1. */
function readLine(record: JsonRecord, key: string, where: string): number {
  const line = readCount(record, key, where);
  if (line < 1) return fail('bad_range', `${where}.${key}: lines count from 1`);
  return line;
}

/** A span's start and end lines, the end not before the start. */
function readSpan(record: JsonRecord, where: string): { start_line: number; end_line: number } {
  const start = readLine(record, 'start_line', where);
  const end = readLine(record, 'end_line', where);
  if (end < start) return fail('bad_range', `${where}.end_line: before its start`);
  return { start_line: start, end_line: end };
}

/** The field as a number of work orders read: at most `MAX_TOLD`, and from `least`. */
function readOrders(record: JsonRecord, key: string, least: number, where: string): number {
  const orders = readCount(record, key, where);
  if (orders < least || orders > MAX_TOLD) {
    return fail('bad_range', `${where}.${key}: from ${least} to ${MAX_TOLD} work orders`);
  }
  return orders;
}

function parseSymbol(value: unknown, index: number): FileSymbol {
  const where = `symbols[${index}]`;
  const record = readRecord(value, where);
  const name = readString(record, 'name', where);
  if (name.length === 0) return fail('bad_shape', `${where}.name: expected a name`);
  return {
    name,
    kind: readEnum(record, 'kind', SYMBOL_KINDS, where),
    ...readSpan(record, where),
    is_nested: readBoolean(record, 'is_nested', where),
    selected: readOrders(record, 'selected', 0, where),
  };
}

function parseSelected(value: unknown, index: number): SelectedSpan {
  const where = `selected[${index}]`;
  const record = readRecord(value, where);
  return { ...readSpan(record, where), orders: readOrders(record, 'orders', 1, where) };
}

/** The count of `key`, which a capped list of `listed` entries is the first `cap` of. */
function readTotal(record: JsonRecord, key: string, listed: number, cap: number): number {
  const total = readCount(record, key, 'file');
  if (listed !== Math.min(total, cap)) {
    return fail('bad_range', `file.${key}: ${total} in all, but ${listed} listed`);
  }
  return total;
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
    const symbols = parseOrdered(
      record,
      'symbols',
      parseSymbol,
      (a, b) => a.start_line <= b.start_line,
      'symbols in line order',
    );
    const selected = parseOrdered(
      record,
      'selected',
      parseSelected,
      (a, b) =>
        a.start_line < b.start_line || (a.start_line === b.start_line && a.end_line < b.end_line),
      'selected spans by their lines, each once',
    );
    checkMaxLength(history, MAX_HISTORY, 'file.history');
    checkMaxLength(told, MAX_TOLD, 'file.told');
    checkMaxLength(symbols, MAX_SYMBOLS, 'file.symbols');
    return {
      version: API_VERSION,
      id,
      path: readPath(record, 'path', 'file'),
      facts,
      history,
      history_total: readTotal(record, 'history_total', history.length, MAX_HISTORY),
      history_reader: readEnum(record, 'history_reader', HISTORY_READERS, 'file'),
      symbols,
      symbols_total: readTotal(record, 'symbols_total', symbols.length, MAX_SYMBOLS),
      selected,
      told,
      editor_url: readEditorUrl(record),
    };
  };
}

/**
 * The detail's editor link: null when it is turned off, and null too when it is longer than
 * `MAX_EDITOR_URL_CHARS`, so a link too long to follow costs the link alone, not the detail.
 */
function readEditorUrl(record: JsonRecord): string | null {
  if (record['editor_url'] === null) return null;
  const url = readString(record, 'editor_url', 'file');
  return codePointLength(url) > MAX_EDITOR_URL_CHARS ? null : url;
}

/** Checks parsed JSON against the file contract for the id `askedId`. */
export function parseFileDetail(value: unknown, askedId: number): ApiResult<FileDetail> {
  return runParse(detailParser(askedId), value);
}

/** Fetches `api/file/<id>` and checks it; an id that is not a file comes back as `not_found`. */
export function fetchFileDetail(id: number, signal: AbortSignal): Promise<ApiResult<FileDetail>> {
  return fetchChecked(fileUrl(id), signal, (value) => parseFileDetail(value, id));
}
