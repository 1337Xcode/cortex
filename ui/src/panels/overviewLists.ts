// The overview panel's lists, each the strongest few of the repository with how many there are,
// and each row a file the map can fly to: the strongest pairs of files changed together (by
// strength times ln(1 + commits), as the daemon ranks them), the busiest files, and the files that
// need attention (a suspect fact). Pure functions of the snapshot; ties break by path so the same
// snapshot always lists the same rows.
import type { GraphFile, GraphLink, GraphSnapshot } from '../api/graph';
import { compareBytes } from '../text/byteOrder';
import { formatCount } from '../text/format';
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
  readonly files: TopList<GraphFile>;
}

/**
 * How strongly a co-change pair changes together, as the daemon ranks co-change links and the
 * summary its strongest pair: the link's weight times ln(1 + commits), so that two files that
 * changed together in 62 commits outrank two whose only 2 commits agree.
 */
export function pairRank(link: Pick<GraphLink, 'weight' | 'commits'>): number {
  return link.weight * Math.log1p(link.commits);
}

/** A list cut to its first rows, and how many rows there were before the cut. */
export interface TopList<T> {
  readonly rows: readonly T[];
  readonly total: number;
}

/** What a section says it holds: the full count, or "top 25 of 2,194" when the list is cut. */
export function listCount(list: TopList<unknown>): string {
  return list.rows.length === list.total
    ? formatCount(list.total)
    : `top ${formatCount(list.rows.length)} of ${formatCount(list.total)}`;
}

/**
 * The strongest co-change pairs, in the graph's own order: by `pairRank`, then commits, then the
 * ids of the two files; at most `LIST.most`, with how many pairs the map links.
 */
export function coupledPairs(snapshot: GraphSnapshot): TopList<CoupledPair> {
  const pairs: CoupledPair[] = [];
  for (const link of snapshot.links) {
    const from = snapshot.files[link.from];
    const to = snapshot.files[link.to];
    if (link.kind !== 'co_change' || from === undefined || to === undefined) continue;
    pairs.push({ link, from, to });
  }
  // INVARIANT: deterministic (ties by commits, then by the files' ids)
  pairs.sort(
    (a, b) =>
      pairRank(b.link) - pairRank(a.link) ||
      b.link.commits - a.link.commits ||
      a.link.from - b.link.from ||
      a.link.to - b.link.to,
  );
  return { rows: pairs.slice(0, LIST.most), total: pairs.length };
}

/**
 * Files ranked by `value`, most first, ties by path, leaving out files whose value is 0; at most
 * `LIST.most`, with how many files have a value.
 */
function rankBy(
  files: readonly GraphFile[],
  value: (file: GraphFile) => number,
): TopList<GraphFile> {
  const ranked = files
    .filter((file) => value(file) > 0)
    .sort((a, b) => value(b) - value(a) || compareBytes(a.path, b.path));
  return { rows: ranked.slice(0, LIST.most), total: ranked.length };
}

/**
 * The files agents spent the most tokens on, or, before any were spent, the most changed files
 * (a generated or vendored file, whose commits are not counted, is never among them).
 */
export function busiestFiles(snapshot: GraphSnapshot): BusiestFiles {
  const byTokens = rankBy(snapshot.files, (file) => file.tokens);
  if (byTokens.total > 0) return { measure: 'tokens', files: byTokens };
  return { measure: 'commits', files: rankBy(snapshot.files, (file) => file.commits) };
}

/** The files with a suspect fact, the most suspect facts first. */
export function filesNeedingAttention(snapshot: GraphSnapshot): TopList<GraphFile> {
  return rankBy(snapshot.files, (file) => file.suspect);
}
