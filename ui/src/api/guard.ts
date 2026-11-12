// The pieces every guard of the daemon's JSON is built from: the typed error, the result type,
// and readers that either return a checked value or stop the parse with a precise message. Each
// endpoint module composes these into one parser; nothing past the parsers checks shapes again.

/** The API version this page understands, the same for every endpoint. */
export const API_VERSION = 1;

/** Why a response could not be used. */
export type ApiErrorCode =
  | 'network'
  | 'http_status'
  | 'not_found'
  | 'bad_json'
  | 'bad_version'
  | 'bad_shape'
  | 'bad_range'
  | 'bad_order'
  | 'bad_kind';

/** A typed failure; `message` is plain text for display and names the offending field. */
export interface ApiError {
  readonly code: ApiErrorCode;
  readonly message: string;
}

/** The outcome of parsing or fetching one response. */
export type ApiResult<T> =
  { readonly ok: true; readonly value: T } | { readonly ok: false; readonly error: ApiError };

/** A JSON object as parsed, before any field is checked. */
export type JsonRecord = Readonly<Record<string, unknown>>;

/** Raised by the readers only; `runParse` turns it into an `ApiResult`. */
class ParseFailure extends Error {
  readonly code: ApiErrorCode;

  constructor(code: ApiErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

/** Stops the parse with `code` and a message naming the field. */
export function fail(code: ApiErrorCode, message: string): never {
  throw new ParseFailure(code, message);
}

/** Runs a parser and turns a refusal into a typed error; any other exception is a bug and is rethrown. */
export function runParse<T>(parse: (value: unknown) => T, value: unknown): ApiResult<T> {
  try {
    return { ok: true, value: parse(value) };
  } catch (error) {
    if (error instanceof ParseFailure) {
      return { ok: false, error: { code: error.code, message: error.message } };
    }
    throw error;
  }
}

/** The longest piece of an unexpected value an error message quotes, so a hostile value cannot flood the page. */
const MAX_QUOTED_CHARS = 40;

/** A value cut to `MAX_QUOTED_CHARS` for an error message. */
export function quote(value: string): string {
  return `"${value.slice(0, MAX_QUOTED_CHARS)}"`;
}

function isRecord(value: unknown): value is JsonRecord {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

/** The value as an object, or a `bad_shape` refusal. */
export function readRecord(value: unknown, where: string): JsonRecord {
  if (!isRecord(value)) return fail('bad_shape', `${where}: expected an object`);
  return value;
}

/** The field as an array, or a `bad_shape` refusal. */
export function readArray(record: JsonRecord, key: string, where: string): readonly unknown[] {
  const value = record[key];
  if (!Array.isArray(value)) return fail('bad_shape', `${where}.${key}: expected an array`);
  return value;
}

/** Checks the `version` field against `API_VERSION`. */
export function readVersion(record: JsonRecord, where: string): typeof API_VERSION {
  const version = record['version'];
  if (version !== API_VERSION) {
    const got = version === undefined ? 'nothing' : JSON.stringify(version);
    return fail('bad_version', `${where}.version: expected ${API_VERSION}, got ${got}`);
  }
  return API_VERSION;
}

/** The field as a non-negative safe integer, or a `bad_shape` refusal. */
export function readCount(record: JsonRecord, key: string, where: string): number {
  const value = record[key];
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    return fail('bad_shape', `${where}.${key}: expected a non-negative integer`);
  }
  return value;
}

/** The latest moment a JavaScript date can hold, in milliseconds since the epoch (year 275760). */
export const MAX_EPOCH_MS = 8.64e15;

/** The field as milliseconds since the epoch, from 0 to `MAX_EPOCH_MS`; past it, `bad_range`. */
export function readEpochMs(record: JsonRecord, key: string, where: string): number {
  const value = readCount(record, key, where);
  if (value > MAX_EPOCH_MS) return fail('bad_range', `${where}.${key}: past the latest date`);
  return value;
}

/** The field as seconds since the epoch, from 0 to `MAX_EPOCH_MS / 1000`; past it, `bad_range`. */
export function readEpochSeconds(record: JsonRecord, key: string, where: string): number {
  const value = readCount(record, key, where);
  if (value > MAX_EPOCH_MS / 1000)
    return fail('bad_range', `${where}.${key}: past the latest date`);
  return value;
}

/** The field as text of at most `maxChars` characters; longer is `bad_range`. */
export function readCappedString(
  record: JsonRecord,
  key: string,
  maxChars: number,
  where: string,
): string {
  const value = readString(record, key, where);
  if (codePointLength(value) > maxChars) {
    return fail('bad_range', `${where}.${key}: longer than ${maxChars} characters`);
  }
  return value;
}

/** The field as a count, or null when the field is null (a missing field is refused). */
export function readNullableCount(record: JsonRecord, key: string, where: string): number | null {
  return record[key] === null ? null : readCount(record, key, where);
}

/** The field as a number from 0 to 1, or a `bad_shape` refusal. */
export function readUnit(record: JsonRecord, key: string, where: string): number {
  const value = record[key];
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) {
    return fail('bad_shape', `${where}.${key}: expected a number from 0 to 1`);
  }
  return value;
}

/** The field as a boolean, or a `bad_shape` refusal. */
export function readBoolean(record: JsonRecord, key: string, where: string): boolean {
  const value = record[key];
  if (typeof value !== 'boolean')
    return fail('bad_shape', `${where}.${key}: expected true or false`);
  return value;
}

/** The field as a boolean or null (a missing field is refused). */
export function readNullableBoolean(
  record: JsonRecord,
  key: string,
  where: string,
): boolean | null {
  return record[key] === null ? null : readBoolean(record, key, where);
}

/** The field as a string, or a `bad_shape` refusal. */
export function readString(record: JsonRecord, key: string, where: string): string {
  const value = record[key];
  if (typeof value !== 'string') return fail('bad_shape', `${where}.${key}: expected a string`);
  return value;
}

/** The field as one of `allowed`; a string outside the set is `bad_kind`, anything else `bad_shape`. */
export function readEnum<T extends string>(
  record: JsonRecord,
  key: string,
  allowed: readonly T[],
  where: string,
): T {
  const value = readString(record, key, where);
  const found = allowed.find((option) => option === value);
  if (found === undefined)
    return fail('bad_kind', `${where}.${key}: unknown value ${quote(value)}`);
  return found;
}

/** The number of Unicode code points in `text`, which is how the daemon counts characters. */
export function codePointLength(text: string): number {
  return Array.from(text).length;
}

/**
 * Characters that change how text around them is shown without being seen: zero-width marks,
 * bidirectional embeddings, overrides and isolates, and the byte-order mark. A path holding one
 * can show one name and be another, so paths refuse them and other text shows them visibly.
 */
const HIDDEN_FORMATTING = /[\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/;

/** The same characters, every occurrence. */
const HIDDEN_FORMATTING_ALL = /[\u200b-\u200f\u202a-\u202e\u2066-\u2069\ufeff]/g;

/** What a hidden formatting character is shown as: the replacement character. */
export const HIDDEN_PLACEHOLDER = '\ufffd';

/** `text` with every hidden formatting character replaced by `HIDDEN_PLACEHOLDER`. */
export function visibleText(text: string): string {
  return text.replace(HIDDEN_FORMATTING_ALL, HIDDEN_PLACEHOLDER);
}

/**
 * Whether `path` is a repository path as the daemon writes them: relative, `/`-separated, with
 * no empty, `.` or `..` segment, no backslash, no control character and no hidden formatting
 * character (see `HIDDEN_FORMATTING`).
 */
export function isRepoPath(path: string): boolean {
  // eslint-disable-next-line no-control-regex -- control characters are exactly what is refused
  if (path.length === 0 || /[\\\u0000-\u001f\u007f]/.test(path)) return false;
  if (HIDDEN_FORMATTING.test(path)) return false;
  return path.split('/').every((segment) => segment !== '' && segment !== '.' && segment !== '..');
}

/** The field as a repository path (see `isRepoPath`), or a `bad_shape` refusal. */
export function readPath(record: JsonRecord, key: string, where: string): string {
  const value = record[key];
  if (typeof value !== 'string' || !isRepoPath(value)) {
    return fail('bad_shape', `${where}.${key}: expected a relative path separated by /`);
  }
  return value;
}

/** Refuses a list longer than `max` with `bad_range`. */
export function checkMaxLength(list: readonly unknown[], max: number, where: string): void {
  if (list.length > max) fail('bad_range', `${where}: ${list.length} entries, at most ${max}`);
}
