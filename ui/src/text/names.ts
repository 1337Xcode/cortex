// Short names for files shown side by side. A file is named by its base name; when two different
// shown files share one, each says which folder it is in ("mod.rs in orders"), and the whole
// parent path when those folders share a name too. Files at the top level are "in the root".
import { baseName } from './format';

function parentOf(path: string): string {
  const slash = path.lastIndexOf('/');
  return slash < 0 ? '' : path.slice(0, slash);
}

function inFolder(path: string, useWholePath: boolean): string {
  const folder = parentOf(path);
  const label = folder === '' ? 'the root' : useWholePath ? folder : baseName(folder);
  return `${baseName(path)} in ${label}`;
}

function counts(values: readonly string[]): Map<string, number> {
  const seen = new Map<string, number>();
  for (const value of values) seen.set(value, (seen.get(value) ?? 0) + 1);
  return seen;
}

/** A name for each path, in order, that tells apart every two different paths in the list. */
export function distinctNames(paths: readonly string[]): string[] {
  const unique = [...new Set(paths)];
  const baseCounts = counts(unique.map(baseName));
  const shortCounts = counts(unique.map((path) => inFolder(path, false)));
  const nameOf = new Map(
    unique.map((path) => {
      if ((baseCounts.get(baseName(path)) ?? 0) < 2) return [path, baseName(path)];
      const short = inFolder(path, false);
      return [path, (shortCounts.get(short) ?? 0) < 2 ? short : inFolder(path, true)];
    }),
  );
  return paths.map((path) => nameOf.get(path) ?? path);
}
