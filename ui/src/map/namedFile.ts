// A file an answer of the daemon names, by its id and its path, as the snapshot on screen holds
// it. Ids move when a file is added or removed, so an id counts only where the snapshot holds the
// same path; a file the snapshot does not hold yet is shown by its path, never linked to whatever
// file sits at its id now.
import { fileIdOf, type GraphFile } from '../api/graph';

/** What the snapshot on screen makes of a file an answer named. */
export type NamedFile =
  /** The snapshot lists it, at this id. */
  | { readonly kind: 'listed'; readonly id: number; readonly path: string }
  /** The daemon lists it, but the snapshot on screen is older and does not hold it yet. */
  | { readonly kind: 'pending'; readonly path: string }
  /** Cortex holds its path, but the map does not list it. */
  | { readonly kind: 'unlisted'; readonly path: string }
  /** The answer named no path, so its id cannot be checked. */
  | { readonly kind: 'unknown' };

/** The file named by `id` (the daemon's id, or null) and `path`, against the snapshot's `files`. */
export function nameFile(
  files: readonly Pick<GraphFile, 'path'>[],
  id: number | null,
  path: string | null,
): NamedFile {
  if (path === null) return { kind: 'unknown' };
  if (id !== null && files[id]?.path === path) return { kind: 'listed', id, path };
  // The snapshot is older or newer than the daemon's ids: the same path may sit at another id.
  const found = fileIdOf(files, path);
  if (found !== null) return { kind: 'listed', id: found, path };
  return id === null ? { kind: 'unlisted', path } : { kind: 'pending', path };
}

/**
 * Whether the daemon named a file by an id the snapshot on screen holds another file at, or no
 * file: the snapshot is older than the daemon's ids, and a fresh one would show it as it is.
 */
export function isOutOfStep(
  files: readonly Pick<GraphFile, 'path'>[],
  id: number | null,
  path: string | null,
): boolean {
  return id !== null && path !== null && files[id]?.path !== path;
}

/** The listed file's id, or null for any other. */
export function listedId(named: NamedFile): number | null {
  return named.kind === 'listed' ? named.id : null;
}

/** The named file's path, or null when the answer gave none. */
export function namedPath(named: NamedFile): string | null {
  return named.kind === 'unknown' ? null : named.path;
}
