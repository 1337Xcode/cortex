// The overview lists on a four-file snapshot whose order is worked out by hand in each test, and the
// count a cut list says in its section's title.
import { describe, expect, it } from 'vitest';
import type { GraphFile, GraphLink, GraphSnapshot } from '../api/graph';
import { KEPT_LINKS_NOTE, keptLinksText } from '../text/vocabulary';
import { busiestFiles, coupledPairs, filesNeedingAttention, listCount } from './overviewLists';

function file(id: number, path: string, fields: Partial<GraphFile>): GraphFile {
  const base = { lines: 10, interest: 0, tokens: 0, touched_ms: null, facts: 0, suspect: 0 };
  return { id, path, ...base, reads: 0, commits: 0, commits_counted: true, ...fields };
}

/** A co-change link over `commits` commits, its count the same. */
function coChange(from: number, to: number, weight: number, commits: number): GraphLink {
  return { from, to, kind: 'co_change', weight, commits, count: commits };
}

const FILES = [
  file(0, 'a.rs', { commits: 3 }),
  file(1, 'b.rs', { tokens: 500, commits: 1, suspect: 2 }),
  file(2, 'c.rs', { tokens: 500, commits: 9, suspect: 1 }),
  file(3, 'd.rs', {}),
];

const LINKS: GraphLink[] = [
  coChange(0, 1, 0.5, 4),
  coChange(0, 2, 0.9, 4),
  coChange(1, 2, 0.3, 7),
  { from: 0, to: 3, kind: 'used_together', weight: 0.8, commits: 0, count: 2 },
];

function snapshot(files: GraphFile[]): GraphSnapshot {
  return { version: 1, files, links: LINKS };
}

/** The pairs' names in their order. */
function pairNames(links: GraphLink[]): string[] {
  return coupledPairs({ version: 1, files: FILES, links }).rows.map(
    (pair) => `${pair.from.path}+${pair.to.path}`,
  );
}

describe('overview lists', () => {
  it('lists co-change pairs by rank, then commits, then ids; other kinds are left out', () => {
    // Ranks, weight times ln(1 + commits): a+c 0.9 ln 5 = 1.448, a+b 0.5 ln 5 = 0.805, b+c 0.3 ln 8
    // = 0.624. The used-together a+d (0.8) is no co-change pair.
    expect(pairNames(LINKS)).toEqual(['a.rs+c.rs', 'a.rs+b.rs', 'b.rs+c.rs']);
  });

  it('breaks equal weights by commits, then by the files ids', () => {
    // Three pairs of weight 0.5: a+c and b+c with 4 commits, a+b with 6. a+b first (more
    // commits); a+c before b+c (from 0 before from 1).
    const links = [coChange(1, 2, 0.5, 4), coChange(0, 2, 0.5, 4), coChange(0, 1, 0.5, 6)];
    expect(pairNames(links)).toEqual(['a.rs+b.rs', 'a.rs+c.rs', 'b.rs+c.rs']);
  });

  it('puts a frequent pair before a perfect pair of two commits', () => {
    // a+b at weight 0.5 over 62 commits ranks 0.5 ln 63 = 2.072; a+c at 1.0 over 2 ranks ln 3 =
    // 1.099, so a+b comes first though it weighs less.
    expect(pairNames([coChange(0, 2, 1, 2), coChange(0, 1, 0.5, 62)])).toEqual([
      'a.rs+b.rs',
      'a.rs+c.rs',
    ]);
  });

  it('ranks the busiest files by tokens, ties by path', () => {
    // b and c both have 500 tokens; b comes first by path; a and d have none.
    const busiest = busiestFiles(snapshot(FILES));
    expect(busiest.measure).toBe('tokens');
    expect(busiest.files.rows.map((entry) => entry.path)).toEqual(['b.rs', 'c.rs']);
    expect(busiest.files.total).toBe(2);
  });

  it('ranks by commits when agents have spent no tokens yet, never a file whose commits are not counted', () => {
    // Commits: c 9, a 3, b 1; d has none and is left out, and so is the generated d.lock, whose
    // commits are not counted (0, as the daemon sends them).
    const cold = [
      ...FILES.map((entry) => ({ ...entry, tokens: 0 })),
      file(4, 'd.lock', { commits_counted: false }),
    ];
    const busiest = busiestFiles(snapshot(cold));
    expect(busiest.measure).toBe('commits');
    expect(busiest.files.rows.map((entry) => entry.path)).toEqual(['c.rs', 'a.rs', 'b.rs']);
  });

  it('lists the files with a suspect fact, the most first', () => {
    expect(filesNeedingAttention(snapshot(FILES)).rows.map((entry) => entry.path)).toEqual([
      'b.rs',
      'c.rs',
    ]);
  });
});

describe('a cut list', () => {
  it('says "top 25 of 30" for 30 co-change links, and the full count when nothing is cut', () => {
    // 30 files in a chain, 0-1, 1-2, ..., 29-30: 30 co-change pairs; the list keeps 25.
    const files = Array.from({ length: 31 }, (_, id) =>
      file(id, `f${String(id).padStart(2, '0')}.rs`, {}),
    );
    const links = Array.from({ length: 30 }, (_, at) => coChange(at, at + 1, 0.5, 3));
    const top = coupledPairs({ version: 1, files, links });
    expect(top.rows).toHaveLength(25);
    expect(top.total).toBe(30);
    expect(listCount(top)).toBe('top 25 of 30');
    expect(listCount(coupledPairs(snapshot(FILES)))).toBe('3');
  });

  it('writes both numbers with thousands separators', () => {
    expect(listCount({ rows: new Array<number>(25).fill(0), total: 2194 })).toBe('top 25 of 2,194');
  });

  it('says a cut list of links counts only the links the map keeps', () => {
    expect(keptLinksText(8, 12)).toBe('top 8 of the 12 the map keeps');
    expect(KEPT_LINKS_NOTE).toBe(
      "Counted among the links the map keeps: each file's 8 strongest of each kind.",
    );
  });
});
