// The words of the activity: each event an agent's hook reported as a sentence ("Claude Code 3f2a…
// edited book.rs"), how long before the list was read it happened ("2 min ago"), and when exactly,
// in UTC. "Ago" counts back from the moment the page read the list, by this computer's clock; the
// events' own times are the clocks of the hooks that reported them, and the section says both.
import type { ActivityEvent, EventKind } from '../api/activity';
import { visibleText } from '../api/guard';
import { formatUtcMoment, formatUtcTime } from './format';
import { shortSessionId } from './liveWords';

/**
 * What an event's agent did when the event names no file the map lists. With a file, a read and an
 * edit name it as their object ("read book.rs") and any other kind says it was about it.
 */
const DID: Readonly<Record<EventKind, string>> = {
  session_start: 'started a session',
  prompt: 'was given a prompt',
  read: 'read a file the map does not list',
  search: 'searched the files',
  edit: 'edited a file the map does not list',
  shell: 'ran a shell command',
  tool: 'used a tool',
  compact: 'compacted its conversation',
  stop: 'finished answering',
  subagent_start: 'started a subagent',
  subagent_stop: 'saw its subagent finish',
  session_end: 'ended its session',
};

/** Who did it: the agent's title and the session's short id, or "Session 3f2a…" without one. */
export function actorName(event: Pick<ActivityEvent, 'agent' | 'session'>): string {
  const id = shortSessionId(event.session);
  return event.agent === null ? `Session ${id}` : `${visibleText(event.agent.title)} ${id}`;
}

/** What happened, without its time: "Claude Code 3f2a… edited book.rs". */
export function eventLine(event: ActivityEvent, fileName: string | null): string {
  return `${actorName(event)} ${didText(event.kind, fileName)}`;
}

/** What was done, with the file when there is one: "edited book.rs", "ran a shell command, about tax.rs". */
function didText(kind: EventKind, fileName: string | null): string {
  if (fileName === null) return DID[kind];
  if (kind === 'read') return `read ${fileName}`;
  if (kind === 'edit') return `edited ${fileName}`;
  return `${DID[kind]}, about ${fileName}`;
}

const SECOND_MS = 1000;
const MINUTE_MS = 60 * SECOND_MS;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/**
 * How long before `readMs` (when the list was read) `atMs` was, short, rounded down: "12 s ago",
 * "2 min ago", "3 h ago", "4 d ago"; "just now" under a second, "later" for a time past `readMs`
 * (a hook's clock ahead of this computer's).
 */
export function shortAgo(atMs: number, readMs: number): string {
  const span = readMs - atMs;
  if (span < 0) return 'later';
  if (span < SECOND_MS) return 'just now';
  if (span < MINUTE_MS) return `${Math.floor(span / SECOND_MS)} s ago`;
  if (span < HOUR_MS) return `${Math.floor(span / MINUTE_MS)} min ago`;
  if (span < DAY_MS) return `${Math.floor(span / HOUR_MS)} h ago`;
  return `${Math.floor(span / DAY_MS)} d ago`;
}

/**
 * The whole event in one sentence, both times, with the file's whole path: "… edited
 * src/orders/book.rs, 2 min ago, at 2026-… UTC."
 */
export function eventSentence(event: ActivityEvent, path: string | null, readMs: number): string {
  return `${eventLine(event, path)}, ${shortAgo(event.at_ms, readMs)}, at ${formatUtcMoment(event.at_ms)}.`;
}

/** The note that says how the times are read. */
export function activityTimesNote(readMs: number): string {
  return `"Ago" counts back from ${formatUtcTime(readMs)} UTC, when this list was read, by this computer's clock; hover a row for its time by its hook's clock.`;
}
