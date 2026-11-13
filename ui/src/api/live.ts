// The live state the daemon pushes over `api/live`: every agent session running in the repository,
// its agent, and the files it worked on most recently. Each frame is the whole state, never a
// change to the last one, so a frame is checked here on its own and either replaces what the page
// knows or is dropped. Everything past this module trusts the types.
import { compareBytes } from '../text/byteOrder';
import {
  API_VERSION,
  checkMaxLength,
  codePointLength,
  fail,
  readArray,
  readCappedString,
  readCount,
  readEnum,
  readEpochMs,
  readRecord,
  readVersion,
  runParse,
  type ApiResult,
  type JsonRecord,
} from './guard';

/** Where the live state is pushed from, relative to the page so the access token is kept. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const LIVE_URL = 'api/live';

/** The most files a session lists, newest first. */
export const MAX_LIVE_FILES = 10;

/**
 * The most sessions a frame may hold. The daemon sends only live sessions of one repository, so a
 * frame past this is not one it sends; refusing it keeps a broken frame from flooding the canvas.
 */
export const MAX_LIVE_SESSIONS = 256;

/**
 * The longest session id the page accepts, in characters: the daemon keeps ids of at most 256
 * bytes, which are never more than 256 characters.
 */
export const MAX_SESSION_ID_CHARS = 256;

/** The longest agent title the page accepts, in characters. */
export const MAX_AGENT_TITLE_CHARS = 64;

/** The longest agent name the page accepts, in characters. */
export const MAX_AGENT_NAME_CHARS = 32;

/** What a session did with a file: read it, edited it, or claimed it for its work. */
export const LIVE_MODES = ['read', 'edit', 'claim'] as const;

/** One of `LIVE_MODES`. */
export type LiveMode = (typeof LIVE_MODES)[number];

/** One file a session worked on. */
export interface LiveFile {
  /** The file's id in `api/graph`. */
  readonly file: number;
  readonly mode: LiveMode;
  /** When the session last read, edited or claimed it, in milliseconds since the epoch. */
  readonly last_ms: number;
}

/** One running agent session. */
export interface LiveSession {
  readonly id: string;
  /** The agent's snake-case name, as Cortex knows it (`claude`, `codex`, ...; `other` for the rest). */
  readonly agent: string;
  /** The agent's display name. */
  readonly title: string;
  /** The files it worked on, newest first, at most `MAX_LIVE_FILES`. */
  readonly files: readonly LiveFile[];
}

/** The whole live state: the running sessions, sorted by id. */
export interface LiveState {
  readonly version: typeof API_VERSION;
  readonly sessions: readonly LiveSession[];
}

/** A snake-case name: a lowercase letter, then lowercase letters, digits and underscores. */
const SNAKE_NAME = /^[a-z][a-z0-9_]*$/;

/**
 * A session id as the daemon keeps one: one line with no control character. It is shown as
 * text, shortened by code points, and with any hidden formatting character made visible.
 */
// eslint-disable-next-line no-control-regex -- control characters are exactly what is refused
const SESSION_ID = /^[^\u0000-\u001f\u007f]+$/;

function parseLiveFile(value: unknown, where: string): LiveFile {
  const record = readRecord(value, where);
  return {
    file: readCount(record, 'file', where),
    mode: readEnum(record, 'mode', LIVE_MODES, where),
    last_ms: readEpochMs(record, 'last_ms', where),
  };
}

function parseLiveFiles(record: JsonRecord, where: string): LiveFile[] {
  const list = readArray(record, 'files', where);
  checkMaxLength(list, MAX_LIVE_FILES, `${where}.files`);
  const files = list.map((entry, index) => parseLiveFile(entry, `${where}.files[${index}]`));
  for (let i = 1; i < files.length; i += 1) {
    if ((files[i]?.last_ms ?? 0) > (files[i - 1]?.last_ms ?? 0)) {
      return fail('bad_order', `${where}.files[${i}]: not newest first`);
    }
  }
  return files;
}

/** The field `key` as a session id (see `SESSION_ID`), or a refusal; receipts read one too. */
export function readSessionId(record: JsonRecord, where: string, key = 'id'): string {
  const id = readCappedString(record, key, MAX_SESSION_ID_CHARS, where);
  if (!SESSION_ID.test(id)) return fail('bad_shape', `${where}.${key}: expected one line of text`);
  return id;
}

/** The `agent` field as an agent's snake name, or a refusal; receipts read one too. */
export function readAgent(record: JsonRecord, where: string): string {
  const agent = readCappedString(record, 'agent', MAX_AGENT_NAME_CHARS, where);
  if (!SNAKE_NAME.test(agent)) return fail('bad_shape', `${where}.agent: expected a snake name`);
  return agent;
}

/** The `title` field as an agent's display name, one line, or a refusal; the activity reads one too. */
export function readTitle(record: JsonRecord, where: string): string {
  const title = readCappedString(record, 'title', MAX_AGENT_TITLE_CHARS, where);
  // eslint-disable-next-line no-control-regex -- control characters are exactly what is refused
  if (codePointLength(title) === 0 || /[\u0000-\u001f\u007f]/.test(title)) {
    return fail('bad_shape', `${where}.title: expected a display name`);
  }
  return title;
}

function parseSession(value: unknown, index: number): LiveSession {
  const where = `sessions[${index}]`;
  const record = readRecord(value, where);
  return {
    id: readSessionId(record, where),
    agent: readAgent(record, where),
    title: readTitle(record, where),
    files: parseLiveFiles(record, where),
  };
}

function parseLiveRecord(value: unknown): LiveState {
  const record = readRecord(value, 'live');
  readVersion(record, 'live');
  const list = readArray(record, 'sessions', 'live');
  checkMaxLength(list, MAX_LIVE_SESSIONS, 'live.sessions');
  const sessions = list.map(parseSession);
  for (let i = 1; i < sessions.length; i += 1) {
    if (compareBytes(sessions[i - 1]?.id ?? '', sessions[i]?.id ?? '') >= 0) {
      return fail('bad_order', `sessions[${i}].id: not after the previous id in byte order`);
    }
  }
  return { version: API_VERSION, sessions };
}

/** Checks one parsed frame against the live contract. */
export function parseLive(value: unknown): ApiResult<LiveState> {
  return runParse(parseLiveRecord, value);
}
