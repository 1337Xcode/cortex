// Tests for the map's shared words: each kind of link said by its own count, the commits of a
// generated file never said as a count, what attributed tokens include, and the first-open
// sentence naming all three kinds of link. Every expected sentence is written out.
import { describe, expect, it } from 'vitest';
import type { GraphFile, GraphLink } from '../api/graph';
import { testFile } from '../map/testScene';
import {
  commitsMeaning,
  fileSentence,
  INTRO_SENTENCE,
  LINK_KIND_MEANINGS,
  linkMeasure,
  linkSentence,
  TOKENS_ATTRIBUTED_MEANING,
} from './vocabulary';

const BOOK: GraphFile = testFile(4, 'src/orders/book.rs', { commits: 21 });
const MOD: GraphFile = testFile(5, 'src/orders/mod.rs', { commits: 15 });
const LOCK: GraphFile = testFile(0, 'Cargo.lock', { commits: 0, commits_counted: false });
const LIMITS = { max_commits: 5000, max_age_days: 365, max_files_per_commit: 30 };

function link(kind: GraphLink['kind'], weight: number, count: number): GraphLink {
  return { from: 4, to: 5, kind, weight, commits: kind === 'co_change' ? count : 0, count };
}

describe('link sentences', () => {
  it("say each kind's own count, with both files' whole paths, never one strength for three measures", () => {
    expect(linkSentence(link('co_change', 0.9, 12), BOOK, MOD)).toBe(
      'src/orders/book.rs changed with src/orders/mod.rs in 12 of 21 commits.',
    );
    // A used-together link of 0.006 made of 3 tasks, and a work-order link of 1.00 made of 1 work
    // order: their counts, not "strength 0.01 of 1" and "strength 1.00 of 1".
    expect(linkSentence(link('used_together', 0.006, 3), BOOK, MOD)).toBe(
      'src/orders/book.rs and src/orders/mod.rs were used together by agents in 3 tasks.',
    );
    expect(linkSentence(link('work_order', 1, 1), BOOK, MOD)).toBe(
      'src/orders/book.rs and src/orders/mod.rs were named together in 1 work order.',
    );
    expect(
      [link('co_change', 0.9, 12), link('used_together', 0.5, 1), link('work_order', 0.4, 2)].map(
        linkMeasure,
      ),
    ).toEqual(['12 commits', '1 task', '2 work orders']);
  });

  it('say what strength measures for each kind in the legend', () => {
    expect(LINK_KIND_MEANINGS.co_change).toContain('Strength: the larger share');
    expect(LINK_KIND_MEANINGS.used_together).toContain('Strength: what those tasks added');
    expect(LINK_KIND_MEANINGS.work_order).toContain('Strength: the work orders that named both');
  });
});

describe('commits of a generated or vendored file', () => {
  it('are said not to be counted, never "None of them changed it"', () => {
    expect(commitsMeaning(LOCK, LIMITS)).toBe(
      'Cortex does not count commits of generated or vendored files.',
    );
    expect(fileSentence(LOCK, 'plain')).toBe(
      'Cargo.lock: 100 lines, read 0 times by agents, its commits not counted (generated or vendored).',
    );
    expect(commitsMeaning(LOCK, LIMITS)).not.toContain('None of them changed it');
  });

  it('still say 0 counted commits plainly for a file Cortex counts', () => {
    const quiet = testFile(1, 'src/a.rs', { commits: 0 });
    expect(commitsMeaning(quiet, LIMITS)).toBe(
      'Commits Cortex read that changed this file: the last 5,000 commits within 365 days, leaving out commits that changed more than 30 files. None of them changed it; older changes, and changes in larger commits, are not counted.',
    );
    expect(fileSentence(quiet, 'plain')).toBe(
      'src/a.rs: 100 lines, read 0 times by agents, not changed in the commits Cortex read.',
    );
  });
});

describe('other words', () => {
  it('say that attributed tokens include cache reads', () => {
    expect(TOKENS_ATTRIBUTED_MEANING).toBe(
      'Tokens of agent turns whose last read or edit was this file: the input and output tokens of those responses, cache reads included.',
    );
  });

  it('name all three kinds of link in the first-open sentence', () => {
    expect(INTRO_SENTENCE).toBe(
      'Folders are squares, files are circles on their rings; lines join files that changed in the same commits, that agents used in one task, or that Cortex named in one work order.',
    );
  });
});
