// The files a live session worked on, as the snapshot on screen holds them. A frame names each file
// by its id and its path, and ids move when files are added or removed, so a file counts only where
// the snapshot holds its path: at the id the frame gave, or at another id when the snapshot is
// older or newer than the daemon's. A file the snapshot does not hold is left out, never marked at
// whatever file sits at its id now; a frame that names one asks the map to read its data again.
import type { GraphFile } from '../api/graph';
import type { LiveMode, LiveSession, LiveState } from '../api/live';
import { isOutOfStep, nameFile } from './namedFile';

/** A file of a live session the snapshot holds: its id there, its path and what was done to it. */
export interface ListedLiveFile {
  readonly id: number;
  readonly path: string;
  readonly mode: LiveMode;
}

/** The files of `session` that `files` holds, newest first, each found by its path. */
export function listedLiveFiles(
  session: LiveSession,
  files: readonly Pick<GraphFile, 'path'>[],
): ListedLiveFile[] {
  return session.files.flatMap((entry) => {
    const named = nameFile(files, entry.file, entry.path);
    return named.kind === 'listed' ? [{ id: named.id, path: named.path, mode: entry.mode }] : [];
  });
}

/**
 * Whether `state` names a file by an id `files` holds another file at, or none: the snapshot on
 * screen is older than the daemon's ids.
 */
export function isFrameAhead(files: readonly Pick<GraphFile, 'path'>[], state: LiveState): boolean {
  return state.sessions.some((session) =>
    session.files.some((entry) => isOutOfStep(files, entry.file, entry.path)),
  );
}
