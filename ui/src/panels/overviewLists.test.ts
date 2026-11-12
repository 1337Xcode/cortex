// The overview lists on a four-file snapshot whose order is worked out by hand in each test.
import { describe, expect, it } from 'vitest';
import type { GraphFile, GraphLink, GraphSnapshot } from '../api/graph';
import { busiestFiles, coupledPairs, filesNeedingAttention } from './overviewLists';

function file(id: number, path: string, fields: Partial<GraphFile>): GraphFile {
  const base = { lines: 10, interest: 0, tokens: 0, touched_ms: null, facts: 0, suspect: 0 };
  return { id, path, ...base, reads: 0, commits: 0, ...fields };
}

const FILES = [
  file(0, 'a.rs', { commits: 3 }),
  file(1, 'b.rs', { tokens: 500, commits: 1, suspect: 2 }),
  file(2, 'c.rs', { tokens: 500, commits: 9, suspect: 1 }),
  file(3, 'd.rs', {}),
];

const LINKS: GraphLink[] = [
  { from: 0, to: 1, kind: 'co_change', weight: 0.5, commits: 4 },
  { from: 0, to: 2, kind: 'co_change', weight: 0.9, commits: 4 },
  { from: 1, to: 2, kind: 'co_change', weight: 0.3, commits: 7 },
  { from: 0, to: 3, kind: 'used_together', weight: 0.8, commits: 0 },
];

function snapshot(files: GraphFile[]): GraphSnapshot {
  return { version: 1, files, links: LINKS };
}

describe('overview lists', () => {
  it('lists co-change pairs by rank, then commits, then ids; other kinds are left out', () => {
    // Ranks, weight times ln(1 + commits): a+c 0.9 ln 5 = 1.448, a+b 0.5 ln 5 = 0.805, b+c 0.3 ln 8
    // = 0.624. The used-together a+d (0.8) is no co-change pair.
    const pairs = coupledPairs(snapshot(FILES)).map((pair) => `${pair.from.path}+${pair.to.path}`);
    expect(pairs).toEqual(['a.rs+c.rs', 'a.rs+b.rs', 'b.rs+c.rs']);
  });

  it('breaks equal weights by commits, then by the files ids', () => {
    // Three pairs of weight 0.5: a+c and b+c with 4 commits, a+b with 6. a+b first (more
    // commits); a+c before b+c (from 0 before from 1).
    const links: GraphLink[] = [
      { from: 1, to: 2, kind: 'co_change', weight: 0.5, commits: 4 },
      { from: 0, to: 2, kind: 'co_change', weight: 0.5, commits: 4 },
      { from: 0, to: 1, kind: 'co_change', weight: 0.5, commits: 6 },
    ];
    const pairs = coupledPairs({ version: 1, files: FILES, links }).map(
      (pair) => `${pair.from.path}+${pair.to.path}`,
    );
    expect(pairs).toEqual(['a.rs+b.rs', 'a.rs+c.rs', 'b.rs+c.rs']);
  });

  it('puts a frequent pair before a perfect pair of two commits', () => {
    // a+b at weight 0.5 over 62 commits ranks 0.5 ln 63 = 2.072; a+c at 1.0 over 2 ranks ln 3 =
    // 1.099, so a+b comes first though it weighs less.
    const links: GraphLink[] = [
      { from: 0, to: 2, kind: 'co_change', weight: 1, commits: 2 },
      { from: 0, to: 1, kind: 'co_change', weight: 0.5, commits: 62 },
    ];
    const pairs = coupledPairs({ version: 1, files: FILES, links }).map(
      (pair) => `${pair.from.path}+${pair.to.path}`,
    );
    expect(pairs).toEqual(['a.rs+b.rs', 'a.rs+c.rs']);
  });

  it('ranks the busiest files by tokens, ties by path', () => {
    // b and c both have 500 tokens; b comes first by path; a and d have none.
    const busiest = busiestFiles(snapshot(FILES));
    expect(busiest.measure).toBe('tokens');
    expect(busiest.files.map((entry) => entry.path)).toEqual(['b.rs', 'c.rs']);
  });

  it('ranks by commits when agents have spent no tokens yet', () => {
    // Commits: c 9, a 3, b 1; d has none and is left out.
    const cold = FILES.map((entry) => ({ ...entry, tokens: 0 }));
    const busiest = busiestFiles(snapshot(cold));
    expect(busiest.measure).toBe('commits');
    expect(busiest.files.map((entry) => entry.path)).toEqual(['c.rs', 'a.rs', 'b.rs']);
  });

  it('lists the files with a suspect fact, the most first', () => {
    expect(filesNeedingAttention(snapshot(FILES)).map((entry) => entry.path)).toEqual([
      'b.rs',
      'c.rs',
    ]);
  });
});
