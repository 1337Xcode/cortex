// What agents did lately, as `api/activity` lists it: the newest events agents' hooks reported (a
// prompt, a read, an edit, a search, a shell command, a stop and the rest), newest first, each with
// its time, its session and agent, and the file it was about, by its id and its path. Edits the
// file watcher saw outside any agent are not in it. Nothing of what a tool read or wrote is kept, so
// nothing of it is here.
import { readAgent, readSessionId, readTitle } from './live';
import {
  API_VERSION,
  checkMaxLength,
  fail,
  readArray,
  readEnum,
  readEpochMs,
  readNullableCount,
  readNullablePath,
  readRecord,
  readVersion,
  runParse,
  type ApiResult,
  type JsonRecord,
} from './guard';
import { fetchChecked } from './request';

/** Where the activity lives, relative to the page. */
// INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
export const ACTIVITY_URL = 'api/activity';

/** The most events the daemon reads: its newest. */
export const MAX_EVENTS = 100;

/** What kind of event an agent's hook reported, as the store names them. */
export const EVENT_KINDS = [
  'session_start',
  'prompt',
  'read',
  'search',
  'edit',
  'shell',
  'tool',
  'compact',
  'stop',
  'subagent_start',
  'subagent_stop',
  'session_end',
] as const;

/** One of `EVENT_KINDS`. */
export type EventKind = (typeof EVENT_KINDS)[number];

/** An agent, by its snake name and its title; null when the session is not stored. */
export interface EventAgent {
  readonly name: string;
  readonly title: string;
}

/** One event an agent's hook reported. */
export interface ActivityEvent {
  /** When, in milliseconds since the epoch, by the clock of the hook that reported it. */
  readonly at_ms: number;
  readonly kind: EventKind;
  readonly session: string;
  readonly agent: EventAgent | null;
  /**
   * The file it was about, by its id in the daemon's `api/graph` when it answered; null when it
   * was about no file the map lists. Ids move when files are added, so check it against `path`.
   */
  readonly file: number | null;
  /** That file's path as the store holds it; null when the event was about no file. */
  readonly path: string | null;
}

/** The agents' latest events, newest first. */
export interface Activity {
  readonly version: typeof API_VERSION;
  readonly events: readonly ActivityEvent[];
}

/** The agent's name and title, both or neither. */
function readEventAgent(record: JsonRecord, where: string): EventAgent | null {
  const hasName = record['agent'] !== null;
  const hasTitle = record['title'] !== null;
  if (!hasName && !hasTitle) return null;
  if (hasName !== hasTitle) return fail('bad_shape', `${where}: an agent comes with its title`);
  return { name: readAgent(record, where), title: readTitle(record, where) };
}

function parseEvent(value: unknown, index: number): ActivityEvent {
  const where = `events[${index}]`;
  const record = readRecord(value, where);
  return {
    at_ms: readEpochMs(record, 'at_ms', where),
    kind: readEnum(record, 'kind', EVENT_KINDS, where),
    session: readSessionId(record, where, 'session'),
    agent: readEventAgent(record, where),
    file: readNullableCount(record, 'file', where),
    path: readNullablePath(record, 'path', where),
  };
}

function parseActivityRecord(value: unknown): Activity {
  const record = readRecord(value, 'activity');
  readVersion(record, 'activity');
  const list = readArray(record, 'events', 'activity');
  checkMaxLength(list, MAX_EVENTS, 'activity.events');
  // Listed in the store's order, newest first. Times are the hooks' own clocks, so two events of
  // two agents may arrive with their times out of step; they are not checked against each other.
  return { version: API_VERSION, events: list.map(parseEvent) };
}

/** Checks parsed JSON against the activity contract. */
export function parseActivity(value: unknown): ApiResult<Activity> {
  return runParse(parseActivityRecord, value);
}

/** Fetches `api/activity` from the daemon that served the page and checks it. */
export function fetchActivity(signal: AbortSignal): Promise<ApiResult<Activity>> {
  return fetchChecked(ACTIVITY_URL, signal, parseActivity);
}
