// The graph snapshot: its types, the hand-written guard that parses the daemon's JSON into
// them, and the fetch call. Everything past this module trusts the types, so anything the
// contract does not allow is rejected here with a typed error the page can show.
import { compareBytes } from '../text/byteOrder';
import {
  API_VERSION,
  fail,
  quote,
  readArray,
  readBoolean,
  readCount,
  readEpochMs,
  readPath,
  readRecord,
  readString,
  readUnit,
  readVersion,
  runParse,
  type ApiResult,
  type JsonRecord,
} from './guard';
import { fetchChecked } from './request';

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
  /** Milliseconds since the epoch of the last read or edit by an agent, or null when never touched. */
  readonly touched_ms: number | null;
  readonly facts: number;
  readonly suspect: number;
  /** Times agents read the file. */
  readonly reads: number;
  /** Commits in the mined history that changed the file; 0, and no count, when not `commits_counted`. */
  readonly commits: number;
  /**
   * Whether Cortex counts the file's commits: false for a generated or vendored file, which the
   * history miner leaves out, so its `commits` says nothing about how often it changed.
   */
  readonly commits_counted: boolean;
}

/** A learned link between two files, `from` always below `to`. */
export interface GraphLink {
  readonly from: number;
  readonly to: number;
  readonly kind: LinkKind;
  readonly weight: number;
  /** Commits that changed both files, for `co_change`; always 0 for the other kinds. */
  readonly commits: number;
  /**
   * What the weight was made from, by kind: the commits that changed both files (`co_change`, the
   * same as `commits`), the tasks that used both (`used_together`), or the delivered work orders
   * that named both (`work_order`).
   */
  readonly count: number;
}

/** The whole graph: files sorted by path (id equals index) and links sorted by kind, from, to. */
export interface GraphSnapshot {
  readonly version: typeof API_VERSION;
  readonly files: readonly GraphFile[];
  readonly links: readonly GraphLink[];
}

function isLinkKind(value: unknown): value is LinkKind {
  return LINK_KINDS.some((kind) => kind === value);
}

function parseFile(value: unknown, index: number): GraphFile {
  const where = `files[${index}]`;
  const record = readRecord(value, where);
  const id = readCount(record, 'id', where);
  if (id !== index) return fail('bad_order', `${where}.id: expected ${index}, got ${id}`);
  return {
    id,
    path: readPath(record, 'path', where),
    lines: readCount(record, 'lines', where),
    interest: readUnit(record, 'interest', where),
    tokens: readCount(record, 'tokens', where),
    touched_ms: record['touched_ms'] === null ? null : readEpochMs(record, 'touched_ms', where),
    facts: readCount(record, 'facts', where),
    suspect: readCount(record, 'suspect', where),
    reads: readCount(record, 'reads', where),
    ...readCommits(record, where),
  };
}

/** A file's commits and whether they are counted; an uncounted file's commits must be 0. */
function readCommits(
  record: JsonRecord,
  where: string,
): Pick<GraphFile, 'commits' | 'commits_counted'> {
  const commits = readCount(record, 'commits', where);
  const isCounted = readBoolean(record, 'commits_counted', where);
  if (!isCounted && commits !== 0) {
    return fail('bad_range', `${where}.commits: a file whose commits are not counted has 0`);
  }
  return { commits, commits_counted: isCounted };
}

function parseFiles(record: JsonRecord): GraphFile[] {
  const files: GraphFile[] = [];
  for (const [index, entry] of readArray(record, 'files', 'snapshot').entries()) {
    const file = parseFile(entry, index);
    const previous = files[index - 1];
    if (previous !== undefined && compareBytes(previous.path, file.path) >= 0) {
      return fail('bad_order', `files[${index}].path: not after the previous path in byte order`);
    }
    files.push(file);
  }
  return files;
}

function readEndpoint(record: JsonRecord, key: string, where: string, count: number): number {
  const id = readCount(record, key, where);
  if (id >= count) return fail('bad_range', `${where}.${key}: ${id} is not a file id`);
  return id;
}

function readKind(record: JsonRecord, where: string): LinkKind {
  const kind = readString(record, 'kind', where);
  if (!isLinkKind(kind)) return fail('bad_kind', `${where}.kind: unknown kind ${quote(kind)}`);
  return kind;
}

function parseLink(value: unknown, index: number, fileCount: number): GraphLink {
  const where = `links[${index}]`;
  const record = readRecord(value, where);
  const kind = readKind(record, where);
  const from = readEndpoint(record, 'from', where, fileCount);
  const to = readEndpoint(record, 'to', where, fileCount);
  if (from >= to) return fail('bad_range', `${where}: from must be below to`);
  const commits = readCount(record, 'commits', where);
  if (kind !== 'co_change' && commits !== 0) {
    return fail('bad_range', `${where}.commits: only a co_change link counts commits`);
  }
  const count = readCount(record, 'count', where);
  if (kind === 'co_change' && count !== commits) {
    return fail('bad_range', `${where}.count: a co_change link counts its commits`);
  }
  return { from, to, kind, weight: readUnit(record, 'weight', where), commits, count };
}

function isLinkBefore(a: GraphLink, b: GraphLink): boolean {
  const kindA = LINK_KINDS.indexOf(a.kind);
  const kindB = LINK_KINDS.indexOf(b.kind);
  if (kindA !== kindB) return kindA < kindB;
  return a.from !== b.from ? a.from < b.from : a.to < b.to;
}

function parseLinks(record: JsonRecord, fileCount: number): GraphLink[] {
  const links: GraphLink[] = [];
  for (const [index, entry] of readArray(record, 'links', 'snapshot').entries()) {
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
  readVersion(record, 'snapshot');
  const files = parseFiles(record);
  const links = parseLinks(record, files.length);
  return { version: API_VERSION, files, links };
}

/**
 * The id of the file at `path` in `files`, which the guard keeps sorted by path in byte order, or
 * null when no file sits there. A binary search: ids move when a file is added or removed, paths
 * do not, so a file is found again in a newer snapshot by its path.
 */
export function fileIdOf(files: readonly Pick<GraphFile, 'path'>[], path: string): number | null {
  let low = 0;
  let high = files.length - 1;
  while (low <= high) {
    const middle = (low + high) >>> 1;
    const file = files[middle];
    if (file === undefined) return null;
    const order = compareBytes(file.path, path);
    if (order === 0) return middle;
    if (order < 0) low = middle + 1;
    else high = middle - 1;
  }
  return null;
}

/** Checks parsed JSON against the snapshot contract and returns the typed snapshot or the error. */
export function parseGraph(value: unknown): ApiResult<GraphSnapshot> {
  return runParse(parseSnapshot, value);
}

/** Fetches `api/graph` from the daemon that served the page and checks it. */
export function fetchGraph(signal: AbortSignal): Promise<ApiResult<GraphSnapshot>> {
  return fetchChecked(GRAPH_URL, signal, parseGraph);
}
