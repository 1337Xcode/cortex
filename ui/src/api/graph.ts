// The graph snapshot: its types, the hand-written guard that parses the daemon's JSON into
// them, and the fetch call. Everything past this module trusts the types, so anything the
// contract does not allow is rejected here with a typed error the page can show.
import { compareBytes } from '../text/byteOrder';

/** The snapshot version this page understands. */
export const GRAPH_VERSION = 1;

/** Where the snapshot lives, relative to the page so the access token in the URL is kept. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const GRAPH_URL = 'api/graph';

/** The kinds of learned link, in the order the daemon sorts them. */
export const LINK_KINDS = ['co_change', 'used_together', 'work_order'] as const;

/** One kind of learned link. */
export type LinkKind = (typeof LINK_KINDS)[number];

/** One file of the repository as the map sees it. */
export interface GraphFile {
  readonly id: number;
  readonly path: string;
  readonly lines: number;
  readonly interest: number;
  readonly tokens: number;
  /** Milliseconds since the epoch of the last touch, or null when never touched. */
  readonly touched_ms: number | null;
  readonly facts: number;
  readonly suspect: number;
}

/** A learned link between two files, `from` always below `to`. */
export interface GraphLink {
  readonly from: number;
  readonly to: number;
  readonly kind: LinkKind;
  readonly weight: number;
}

/** The whole graph: files sorted by path (id equals index) and links sorted by kind, from, to. */
export interface GraphSnapshot {
  readonly version: typeof GRAPH_VERSION;
  readonly files: readonly GraphFile[];
  readonly links: readonly GraphLink[];
}

/** Why a snapshot could not be loaded. */
export type GraphErrorCode =
  | 'network'
  | 'http_status'
  | 'bad_json'
  | 'bad_version'
  | 'bad_shape'
  | 'bad_range'
  | 'bad_order'
  | 'bad_kind';

/** A typed load failure; `message` is plain text for display and names the offending field. */
export interface GraphError {
  readonly code: GraphErrorCode;
  readonly message: string;
}

/** The outcome of parsing or fetching a snapshot. */
export type GraphResult =
  | { readonly ok: true; readonly snapshot: GraphSnapshot }
  | { readonly ok: false; readonly error: GraphError };

/** Loads a snapshot; the store passes the signal so a newer load can cancel an older one. */
export type GraphFetcher = (signal: AbortSignal) => Promise<GraphResult>;

/** Raised inside this module only; `parseGraph` turns it into a `GraphResult`. */
class GraphParseError extends Error {
  readonly code: GraphErrorCode;

  constructor(code: GraphErrorCode, message: string) {
    super(message);
    this.code = code;
  }
}

function fail(code: GraphErrorCode, message: string): never {
  throw new GraphParseError(code, message);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isLinkKind(value: unknown): value is LinkKind {
  return LINK_KINDS.some((kind) => kind === value);
}

function readRecord(value: unknown, where: string): Record<string, unknown> {
  if (!isRecord(value)) return fail('bad_shape', `${where}: expected an object`);
  return value;
}

function readCount(record: Record<string, unknown>, key: string, where: string): number {
  const value = record[key];
  if (typeof value !== 'number' || !Number.isSafeInteger(value) || value < 0) {
    return fail('bad_shape', `${where}.${key}: expected a non-negative integer`);
  }
  return value;
}

function readUnit(record: Record<string, unknown>, key: string, where: string): number {
  const value = record[key];
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0 || value > 1) {
    return fail('bad_shape', `${where}.${key}: expected a number from 0 to 1`);
  }
  return value;
}

function readTouched(record: Record<string, unknown>, where: string): number | null {
  return record['touched_ms'] === null ? null : readCount(record, 'touched_ms', where);
}

/** The longest unknown kind an error message quotes, so a hostile value cannot flood the page. */
const MAX_QUOTED_CHARS = 40;

/**
 * Whether `path` is a repository path as the daemon writes them: relative, `/`-separated, with
 * no empty, `.` or `..` segment, no backslash and no control character.
 */
export function isRepoPath(path: string): boolean {
  // eslint-disable-next-line no-control-regex -- control characters are exactly what is refused
  if (path.length === 0 || /[\\\u0000-\u001f\u007f]/.test(path)) return false;
  return path.split('/').every((segment) => segment !== '' && segment !== '.' && segment !== '..');
}

function readPath(record: Record<string, unknown>, where: string): string {
  const value = record['path'];
  if (typeof value !== 'string' || !isRepoPath(value)) {
    return fail('bad_shape', `${where}.path: expected a relative path separated by /`);
  }
  return value;
}

function parseFile(value: unknown, index: number): GraphFile {
  const where = `files[${index}]`;
  const record = readRecord(value, where);
  const id = readCount(record, 'id', where);
  if (id !== index) return fail('bad_order', `${where}.id: expected ${index}, got ${id}`);
  return {
    id,
    path: readPath(record, where),
    lines: readCount(record, 'lines', where),
    interest: readUnit(record, 'interest', where),
    tokens: readCount(record, 'tokens', where),
    touched_ms: readTouched(record, where),
    facts: readCount(record, 'facts', where),
    suspect: readCount(record, 'suspect', where),
  };
}

function parseFiles(value: unknown): GraphFile[] {
  if (!Array.isArray(value)) return fail('bad_shape', 'files: expected an array');
  const files: GraphFile[] = [];
  for (const [index, entry] of value.entries()) {
    const file = parseFile(entry, index);
    const previous = files[index - 1];
    if (previous !== undefined && compareBytes(previous.path, file.path) >= 0) {
      return fail('bad_order', `files[${index}].path: not after the previous path in byte order`);
    }
    files.push(file);
  }
  return files;
}

function readEndpoint(
  record: Record<string, unknown>,
  key: string,
  where: string,
  count: number,
): number {
  const id = readCount(record, key, where);
  if (id >= count) return fail('bad_range', `${where}.${key}: ${id} is not a file id`);
  return id;
}

function parseLink(value: unknown, index: number, fileCount: number): GraphLink {
  const where = `links[${index}]`;
  const record = readRecord(value, where);
  const kind = record['kind'];
  if (typeof kind !== 'string') return fail('bad_shape', `${where}.kind: expected a string`);
  if (!isLinkKind(kind)) {
    return fail('bad_kind', `${where}.kind: unknown kind "${kind.slice(0, MAX_QUOTED_CHARS)}"`);
  }
  const from = readEndpoint(record, 'from', where, fileCount);
  const to = readEndpoint(record, 'to', where, fileCount);
  if (from >= to) return fail('bad_range', `${where}: from must be below to`);
  return { from, to, kind, weight: readUnit(record, 'weight', where) };
}

function isLinkBefore(a: GraphLink, b: GraphLink): boolean {
  const kindA = LINK_KINDS.indexOf(a.kind);
  const kindB = LINK_KINDS.indexOf(b.kind);
  if (kindA !== kindB) return kindA < kindB;
  return a.from !== b.from ? a.from < b.from : a.to < b.to;
}

function parseLinks(value: unknown, fileCount: number): GraphLink[] {
  if (!Array.isArray(value)) return fail('bad_shape', 'links: expected an array');
  const links: GraphLink[] = [];
  for (const [index, entry] of value.entries()) {
    const link = parseLink(entry, index, fileCount);
    const previous = links[index - 1];
    if (previous !== undefined && !isLinkBefore(previous, link)) {
      return fail('bad_order', `links[${index}]: not after the previous link by kind, from, to`);
    }
    links.push(link);
  }
  return links;
}

function parseSnapshot(value: unknown): GraphSnapshot {
  const record = readRecord(value, 'snapshot');
  const version = record['version'];
  if (version !== GRAPH_VERSION) {
    return fail(
      'bad_version',
      `version: expected ${GRAPH_VERSION}, got ${JSON.stringify(version)}`,
    );
  }
  const files = parseFiles(record['files']);
  const links = parseLinks(record['links'], files.length);
  return { version: GRAPH_VERSION, files, links };
}

/** Checks parsed JSON against the snapshot contract and returns the typed snapshot or the error. */
export function parseGraph(value: unknown): GraphResult {
  try {
    return { ok: true, snapshot: parseSnapshot(value) };
  } catch (error) {
    if (error instanceof GraphParseError) {
      return { ok: false, error: { code: error.code, message: error.message } };
    }
    throw error;
  }
}

function describeFailure(cause: unknown): string {
  return cause instanceof Error ? cause.message : 'the request failed';
}

/** Fetches `api/graph` from the daemon that served the page and checks it. */
export async function fetchGraph(
  signal: AbortSignal,
  url: string = GRAPH_URL,
): Promise<GraphResult> {
  let response: Response;
  try {
    response = await fetch(url, {
      signal,
      cache: 'no-store',
      headers: { accept: 'application/json' },
    });
  } catch (cause) {
    return { ok: false, error: { code: 'network', message: describeFailure(cause) } };
  }
  if (!response.ok) {
    const message = `the daemon answered ${response.status}`;
    return { ok: false, error: { code: 'http_status', message } };
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    return { ok: false, error: { code: 'bad_json', message: describeFailure(cause) } };
  }
  return parseGraph(body);
}
