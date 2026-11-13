// The name a file goes by wherever the page names it in a sentence or a row: the shortest end of
// its path, in whole segments, that no other file of the map ends with. A repository holding a
// dozen README.md files names them `cortex-store/README.md`, `ui/README.md` and so on, while a
// file whose name is unique keeps its bare name. A path whose every end is also the end of a longer
// path (the root README.md beside `docs/README.md`, or `src/lib.rs` beside `crates/a/src/lib.rs`)
// is named in full from the root, `./README.md`. A path the map does not hold is named in full.
// The whole path is shown on hover where the name is.
import type { GraphFile } from '../api/graph';
import { baseName } from './format';

/** Names for the files of one map. */
export interface PathNames {
  /** The name `path` goes by: its shortest unshared end, or its whole path when unknown. */
  readonly nameOf: (path: string) => string;
}

/** Whether `other`, a different path, ends with the segments `end`. */
function endsWithSegments(other: string, end: string): boolean {
  return other === end || other.endsWith(`/${end}`);
}

/** The shortest end of `path` that no path of `sameBase` (its base name's paths) shares. */
function nameAmong(path: string, sameBase: readonly string[]): string {
  const segments = path.split('/');
  for (let count = 1; count <= segments.length; count += 1) {
    const end = segments.slice(-count).join('/');
    if (!sameBase.some((other) => other !== path && endsWithSegments(other, end))) return end;
  }
  return `./${path}`;
}

/**
 * The names of `paths` (a map's files). The paths are grouped by base name on the first question
 * and each name is worked out once, when first asked for, so a large map pays only for the names
 * it shows.
 */
export function pathNames(paths: readonly string[]): PathNames {
  let byBase: Map<string, string[]> | null = null;
  const named = new Map<string, string>();
  const groups = (): Map<string, string[]> => {
    if (byBase !== null) return byBase;
    const built = new Map<string, string[]>();
    for (const path of new Set(paths)) {
      const base = baseName(path);
      const group = built.get(base);
      if (group === undefined) built.set(base, [path]);
      else group.push(path);
    }
    byBase = built;
    return built;
  };
  return {
    nameOf: (path) => {
      const known = named.get(path);
      if (known !== undefined) return known;
      const sameBase = groups().get(baseName(path));
      const name = sameBase?.includes(path) === true ? nameAmong(path, sameBase) : path;
      named.set(path, name);
      return name;
    },
  };
}

/** One `PathNames` per list of files, so every panel names a map's files the same way. */
const NAMES_OF_FILES = new WeakMap<readonly Pick<GraphFile, 'path'>[], PathNames>();

/** The names of the files of a snapshot (`files` as the snapshot holds them). */
export function namesOfFiles(files: readonly Pick<GraphFile, 'path'>[]): PathNames {
  const known = NAMES_OF_FILES.get(files);
  if (known !== undefined) return known;
  const names = pathNames(files.map((file) => file.path));
  NAMES_OF_FILES.set(files, names);
  return names;
}
