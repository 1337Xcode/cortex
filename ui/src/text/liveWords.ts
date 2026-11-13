// The words for live sessions: a session's short name ("Claude Code 3f2a…") and the sentence that
// explains its marker on hover and its row on focus ("Claude Code session 3f2a… is editing
// book.rs; before that it read mod.rs and payment.rs").
import type { LiveMode, LiveSession } from '../api/live';
import { visibleText } from '../api/guard';
import { LIVE } from '../theme/tokens';
import { countOf } from './format';
import type { SessionNames } from './sessionNames';

/** What a session is doing with its newest file. */
const DOING: Readonly<Record<LiveMode, string>> = {
  read: 'is reading',
  edit: 'is editing',
  claim: 'has claimed',
};

/** What a session did with an earlier file. */
const DID: Readonly<Record<LiveMode, string>> = {
  read: 'read',
  edit: 'edited',
  claim: 'claimed',
};

/** What a session is doing with a file, in a row's few words. */
const ROW_DOING: Readonly<Record<LiveMode, string>> = {
  read: 'reading',
  edit: 'editing',
  claim: 'claimed',
};

/** A session's name: its agent's title and the name the page gives its id ("Claude Code 3f2a…"). */
export function sessionName(session: LiveSession, names: SessionNames): string {
  return `${visibleText(session.title)} ${names.nameOf(session.id)}`;
}

/** What leads a session's row before its file: its name and what it is doing ("… editing"). */
export function sessionLead(session: LiveSession, mode: LiveMode, names: SessionNames): string {
  return `${sessionName(session, names)} ${ROW_DOING[mode]}`;
}

/** Names joined as a reader says them: "a", "a and b", "a, b and c". */
export function joinNames(names: readonly string[]): string {
  if (names.length <= 1) return names.join('');
  return `${names.slice(0, -1).join(', ')} and ${names.at(-1) ?? ''}`;
}

/** A file a session worked on: the name the map gives it, and what the session did to it. */
export interface NamedLiveFile {
  readonly name: string;
  readonly mode: LiveMode;
}

/** "read mod.rs and payment.rs, and edited tax.rs": earlier files grouped by what was done. */
function earlierPhrase(files: readonly NamedLiveFile[]): string {
  const groups: { mode: LiveMode; names: string[] }[] = [];
  for (const file of files) {
    const group = groups.find((entry) => entry.mode === file.mode);
    if (group === undefined) groups.push({ mode: file.mode, names: [file.name] });
    else group.names.push(file.name);
  }
  return groups.map((group) => `${DID[group.mode]} ${joinNames(group.names)}`).join(', and ');
}

/**
 * The sentence for a session whose files the map shows are `files`, newest first: what it is
 * doing with the newest, then up to `LIVE.namedBefore` earlier files and how many more there are.
 */
export function liveSentence(
  session: LiveSession,
  files: readonly NamedLiveFile[],
  names: SessionNames,
): string {
  const who = `${visibleText(session.title)} session ${names.nameOf(session.id)}`;
  const [newest, ...earlier] = files;
  if (newest === undefined) return `${who} is working on files the map does not show yet.`;
  const now = `${who} ${DOING[newest.mode]} ${newest.name}`;
  if (earlier.length === 0) return `${now}.`;
  const named = earlier.slice(0, LIVE.namedBefore);
  const more = earlier.length - named.length;
  const tail = more > 0 ? `, plus ${countOf(more, 'more file')}` : '';
  return `${now}; before that it ${earlierPhrase(named)}${tail}.`;
}
