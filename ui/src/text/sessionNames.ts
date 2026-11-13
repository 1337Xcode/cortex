// The name an agent session goes by wherever the page names it: the shortest start of its id, at
// least `LIVE.shortIdChars` characters, that no other session the page knows of starts with, with
// an ellipsis when the id was cut. Codex ids are UUIDv7, whose first characters are a timestamp, so
// two sessions begun a minute apart share their first eight characters and a fixed cut would name
// them alike. An id no longer than its start is named whole, and so is an id that is the start of
// another. The page names each session from one `SessionNames` built from every session it holds
// (the live ones, the activity's and the receipts'), so a session reads the same in all of them at
// once. The whole id is shown on hover where the name is.
import { LIVE } from '../theme/tokens';
import { compareBytes } from './byteOrder';

/** Names for the sessions of one page. */
export interface SessionNames {
  /** The name `id` goes by: its shortest unshared start, or the whole id when it is unknown. */
  readonly nameOf: (id: string) => string;
}

/** How many leading characters the two lists of characters share. */
function sharedStart(a: readonly string[], b: readonly string[]): number {
  let count = 0;
  while (count < a.length && count < b.length && a[count] === b[count]) count += 1;
  return count;
}

/** The name of `characters` when its neighbours in id order share their first `shared`. */
function nameFrom(characters: readonly string[], shared: number): string {
  const needed = Math.max(shared + 1, LIVE.shortIdChars);
  if (characters.length <= needed) return characters.join('');
  return `${characters.slice(0, needed).join('')}…`;
}

/**
 * The names of `ids`. The ids are sorted by byte order, and each needs one character more than it
 * shares with the id before it or the id after it, because every other id that shares a longer
 * start sorts next to it. Characters are counted as a reader counts them, so an id is never cut
 * inside a character.
 */
export function sessionNames(ids: Iterable<string>): SessionNames {
  const sorted = [...new Set(ids)].sort(compareBytes);
  const characters = sorted.map((id) => Array.from(id));
  const named = new Map<string, string>();
  for (const [index, id] of sorted.entries()) {
    const own = characters[index] ?? [];
    const before = characters[index - 1] ?? [];
    const after = characters[index + 1] ?? [];
    named.set(id, nameFrom(own, Math.max(sharedStart(own, before), sharedStart(own, after))));
  }
  return { nameOf: (id) => named.get(id) ?? id };
}
