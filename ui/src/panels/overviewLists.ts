// The overview panel's lists, each the strongest few of the repository and each row a file the map
// can fly to: the pairs of files changed together most often, the busiest files, and the files
// that need attention (a suspect fact). Pure functions of the snapshot; ties break by path so the
// same snapshot always lists the same rows.
import type { GraphFile, GraphLink, GraphSnapshot } from '../api/graph';
import { compareBytes } from '../text/byteOrder';
import { LIST } from '../theme/tokens';

/** Two files changed together, with the link between them. */
export interface CoupledPair {
  readonly link: GraphLink;
  readonly from: GraphFile;
  readonly to: GraphFile;
}

/** The busiest files and the number they are ranked by. */
export interface BusiestFiles {
  /** Tokens attributed when any file has some; commits otherwise (nothing measured by agents yet). */
  readonly measure: 'tokens' | 'commits';
  readonly files: readonly GraphFile[];
}

/** The co-change pairs with the most commits, most first; equal counts by weight, then by path. */
export function coupledPairs(snapshot: GraphSnapshot): CoupledPair[] {
  const pairs: CoupledPair[] = [];
  for (const link of snapshot.links) {
    const from = snapshot.files[link.from];
    const to = snapshot.files[link.to];
    if (link.kind !== 'co_change' || from === undefined || to === undefined) continue;
    pairs.push({ link, from, to });
  }
  // INVARIANT: deterministic (ties by weight, then by the byte order of the paths)
  pairs.sort(
    (a, b) =>
      b.link.commits - a.link.commits ||
      b.link.weight - a.link.weight ||
      compareBytes(a.from.path, b.from.path) ||
      compareBytes(a.to.path, b.to.path),
  );
  return pairs.slice(0, LIST.most);
}

/** Files ranked by `value`, most first, ties by path, leaving out files whose value is 0. */
function rankBy(files: readonly GraphFile[], value: (file: GraphFile) => number): GraphFile[] {
  return files
    .filter((file) => value(file) > 0)
    .sort((a, b) => value(b) - value(a) || compareBytes(a.path, b.path))
    .slice(0, LIST.most);
}

/** The files agents spent the most tokens on, or, before any were spent, the most changed files. */
export function busiestFiles(snapshot: GraphSnapshot): BusiestFiles {
  const byTokens = rankBy(snapshot.files, (file) => file.tokens);
  if (byTokens.length > 0) return { measure: 'tokens', files: byTokens };
  return { measure: 'commits', files: rankBy(snapshot.files, (file) => file.commits) };
}

/** The files with a suspect fact, the most suspect facts first. */
export function filesNeedingAttention(snapshot: GraphSnapshot): GraphFile[] {
  return rankBy(snapshot.files, (file) => file.suspect);
}
