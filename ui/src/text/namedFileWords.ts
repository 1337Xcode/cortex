// The words for a file an answer named that the map on screen does not show: its path in full,
// with a short note that says why it is not a link ("not on the map yet" when the map's snapshot
// is older than the answer, "not on the map" when the map does not list it at all).
import type { NamedFile } from '../map/namedFile';

/** The short note a row adds for a file it names by path, or null for a listed or unknown one. */
export function namedFileNote(named: NamedFile): string | null {
  switch (named.kind) {
    case 'pending':
      return 'not on the map yet';
    case 'unlisted':
      return 'not on the map';
    case 'listed':
    case 'unknown':
      return null;
  }
}

/** The note as a sentence for the row's accessible name: "src/new.rs is not on the map yet." */
export function namedFileSentence(named: NamedFile): string | null {
  const note = namedFileNote(named);
  return note === null || named.kind === 'listed' || named.kind === 'unknown'
    ? null
    : `${named.path} is ${note}.`;
}

/**
 * The name each named file goes by in a list: listed files by `names` (their names told apart),
 * any other by its whole path, and null for a file the answer named no path for.
 */
export function shownName(named: NamedFile, names: ReadonlyMap<string, string>): string | null {
  switch (named.kind) {
    case 'listed':
      return names.get(named.path) ?? named.path;
    case 'pending':
    case 'unlisted':
      return named.path;
    case 'unknown':
      return null;
  }
}
