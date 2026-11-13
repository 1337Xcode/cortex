// Search over paths, file names and symbols: the query rules, the URL, and the guard for the
// daemon's answer. A response is accepted only for the exact text that was asked, so a late
// answer to an older query can never be shown as the answer to the newer one. Each result names
// its file by id and by path, so the page can tell a file its snapshot holds at another id.
import {
  API_VERSION,
  checkMaxLength,
  codePointLength,
  fail,
  readArray,
  readCount,
  readEnum,
  readNullableCount,
  readPath,
  readRecord,
  readString,
  readVersion,
  runParse,
  type ApiResult,
} from './guard';
import { fetchChecked } from './request';

/** The longest query the daemon accepts, in characters. */
export const MAX_QUERY_CHARS = 100;

/** The most results the daemon returns. */
export const MAX_SEARCH_RESULTS = 20;

/** What a result names. */
export const SEARCH_KINDS = ['file', 'symbol'] as const;

/** One of `SEARCH_KINDS`. */
export type SearchKind = (typeof SEARCH_KINDS)[number];

/** One search result: a file (no line) or a symbol in a file (with its 1-based line). */
export interface SearchResult {
  readonly kind: SearchKind;
  /**
   * The graph id of the file, or of the file the symbol is in. Ids move when files are added, so
   * check it against `path`.
   */
  readonly file: number;
  /** That file's path, as the daemon's graph lists it. */
  readonly path: string;
  readonly name: string;
  readonly line: number | null;
}

/** The daemon's answer to one query. */
export interface SearchAnswer {
  readonly version: typeof API_VERSION;
  readonly query: string;
  readonly results: readonly SearchResult[];
}

/** Whether `text` is a query the daemon accepts: 1 to `MAX_QUERY_CHARS` characters. */
export function isValidQuery(text: string): boolean {
  const length = codePointLength(text);
  return length >= 1 && length <= MAX_QUERY_CHARS;
}

/** The relative URL for a query, with the text encoded. */
export function searchUrl(text: string): string {
  // INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
  return `api/search?q=${encodeURIComponent(text)}`;
}

function parseResult(value: unknown, index: number): SearchResult {
  const where = `results[${index}]`;
  const record = readRecord(value, where);
  const kind = readEnum(record, 'kind', SEARCH_KINDS, where);
  const name = readString(record, 'name', where);
  if (name.length === 0) return fail('bad_shape', `${where}.name: expected a name`);
  const line = readNullableCount(record, 'line', where);
  if (kind === 'file' && line !== null) {
    return fail('bad_shape', `${where}.line: a file result has no line`);
  }
  if (kind === 'symbol' && (line === null || line < 1)) {
    return fail('bad_shape', `${where}.line: a symbol result needs a line from 1`);
  }
  return {
    kind,
    file: readCount(record, 'file', where),
    path: readPath(record, 'path', where),
    name,
    line,
  };
}

function answerParser(asked: string): (value: unknown) => SearchAnswer {
  return (value) => {
    const record = readRecord(value, 'search');
    readVersion(record, 'search');
    const query = readString(record, 'query', 'search');
    if (query !== asked) return fail('bad_shape', 'search.query: not the text that was asked');
    const entries = readArray(record, 'results', 'search');
    checkMaxLength(entries, MAX_SEARCH_RESULTS, 'search.results');
    return { version: API_VERSION, query, results: entries.map(parseResult) };
  };
}

/** Checks parsed JSON against the search contract for the query `asked`. */
export function parseSearch(value: unknown, asked: string): ApiResult<SearchAnswer> {
  return runParse(answerParser(asked), value);
}

/** Fetches `api/search` for `text`, which must pass `isValidQuery`, and checks the answer. */
export function fetchSearch(text: string, signal: AbortSignal): Promise<ApiResult<SearchAnswer>> {
  return fetchChecked(searchUrl(text), signal, (value) => parseSearch(value, text));
}
