// Tests for the text the panels show: the summary strip's sentences and the side panel's
// strongest links, with every expected sentence written out.
import { describe, expect, it } from 'vitest';
import type { GraphLink } from '../api/graph';
import type { MapSummary } from '../api/summary';
import { sevenFileScene, sevenFileSnapshot, testLink } from '../map/testScene';
import { distinctNames } from '../text/names';
import { strongestLinks } from './fileLinks';
import { detailFor } from './fileSections';
import { summarySentences, type SummarySentence } from './summarySentences';

const LEARNED: MapSummary = {
  version: 1,
  files: 1204,
  folders: 87,
  explored_files: 312,
  learned: true,
  strongest_pair: { from: 1, to: 2, commits: 31 },
  busiest_folder: { path: 'src/orders', tokens: 52310 },
  window_days: 7,
  history: { max_commits: 5000, max_age_days: 365, max_files_per_commit: 30 },
};

/** The seven-file snapshot holding the summary's strongest pair, a/one.rs and a/two.rs. */
const PAIRED = sevenFileSnapshot([
  { from: 1, to: 2, kind: 'co_change', weight: 0.8, commits: 31, count: 31 },
]);

function plain(sentence: SummarySentence): string {
  return sentence.parts
    .map((part) => (typeof part === 'string' ? part : `[${part.path}]`))
    .join('');
}

describe('summarySentences', () => {
  it('says what is indexed, read, coupled and busiest, in plain sentences', () => {
    const text = summarySentences(LEARNED, PAIRED);
    expect(text.ok ? text.sentences.map(plain) : text.error).toEqual([
      'Strongest coupling: [a/one.rs] and [a/two.rs] changed together in 31 commits.',
      'Most worked on in the last 7 days: src/orders/, whose own files took 52,310 tokens attributed.',
      'Agents have read 312 of the 1,204 files (26%).',
      '1,204 files in 87 folders.',
    ]);
  });

  it('names the top folder in words when its own files took the most', () => {
    const top = { ...LEARNED, busiest_folder: { path: '', tokens: 1200 } };
    const text = summarySentences(top, PAIRED);
    const busiest = text.ok ? text.sentences.find((sentence) => sentence.key === 'busiest') : null;
    expect(busiest ? plain(busiest) : null).toBe(
      "Most worked on in the last 7 days: the repository's top folder, whose own files took 1,200 tokens attributed.",
    );
  });

  it('never rounds a share of read files to 0% or 100%', () => {
    // 2 of 1,595 is 0.13%, which rounds to 0; 1,594 of 1,595 is 99.94%, which rounds to 100.
    const share = (read: number): string => {
      const text = summarySentences({ ...LEARNED, files: 1595, explored_files: read }, PAIRED);
      const explored = text.ok ? text.sentences.find((s) => s.key === 'explored') : undefined;
      return explored === undefined ? '' : plain(explored);
    };
    expect(share(2)).toBe('Agents have read 2 of the 1,595 files (under 1%).');
    expect(share(1594)).toBe('Agents have read 1,594 of the 1,595 files (over 99%).');
    expect(share(1595)).toBe('Agents have read 1,595 of the 1,595 files (100%).');
  });

  it('gives each sentence a short form for the one-line strip', () => {
    const text = summarySentences(LEARNED, PAIRED);
    const short = (sentence: SummarySentence): string =>
      plain({ ...sentence, parts: sentence.short });
    expect(text.ok ? text.sentences.map(short) : text.error).toEqual([
      '[a/one.rs] and [a/two.rs]: 31 commits together.',
      'Busiest folder: src/orders/.',
      '26% read by agents.',
      '1,204 files.',
    ]);
  });

  it('says a cold repository shows its structure alone', () => {
    const cold = {
      ...LEARNED,
      files: 1,
      folders: 1,
      explored_files: 0,
      learned: false,
      strongest_pair: null,
      busiest_folder: null,
    };
    const text = summarySentences(cold, sevenFileSnapshot());
    expect(text.ok ? text.sentences.map(plain) : text.error).toEqual([
      'Cortex has not learned anything here yet, so the map shows the structure alone.',
      '1 file in 1 folder.',
    ]);
  });

  it('says when nothing has been read, and leaves out what it cannot name', () => {
    const quiet = { ...LEARNED, explored_files: 0, strongest_pair: null, busiest_folder: null };
    const text = summarySentences(quiet, sevenFileSnapshot());
    expect(text.ok ? text.sentences.map(plain) : text.error).toEqual([
      'Agents have not read any of these files yet.',
      '1,204 files in 87 folders.',
    ]);
  });

  it('refuses a pair naming a file the map does not have', () => {
    const text = summarySentences(
      { ...LEARNED, strongest_pair: { from: 1, to: 7, commits: 2 } },
      sevenFileSnapshot(),
    );
    expect(text.ok ? 'shown' : text.error.code).toBe('bad_range');
  });

  it('refuses a pair the map holds no co-change link for, or holds with other commits', () => {
    // Both files exist, but the map links them only as used together, or with 30 commits, not 31.
    const usedTogether = sevenFileSnapshot([testLink(1, 2, 'used_together', 0.8)]);
    const otherCount = sevenFileSnapshot([
      { from: 1, to: 2, kind: 'co_change', weight: 0.8, commits: 30, count: 30 },
    ]);
    for (const snapshot of [sevenFileSnapshot(), usedTogether, otherCount]) {
      const text = summarySentences(LEARNED, snapshot);
      expect(text.ok ? 'shown' : text.error.code).toBe('bad_range');
    }
  });
});

describe('strongestLinks', () => {
  it('lists each kind strongest first, ties by path, at most the asked number, with each total', () => {
    const scene = sevenFileScene([
      testLink(0, 1, 'co_change', 0.5),
      testLink(0, 2, 'co_change', 0.9),
      testLink(0, 6, 'co_change', 0.5),
      testLink(0, 4, 'work_order', 0.3),
    ]);
    const links = strongestLinks(scene, 0, 2);
    expect(links.shown.co_change.map((link) => link.path)).toEqual(['a/two.rs', 'a/one.rs']);
    expect(links.shown.used_together).toEqual([]);
    expect(links.shown.work_order).toEqual([
      { other: 4, path: 'b/mid.rs', weight: 0.3, commits: 0, count: 1 },
    ]);
    expect(links.totals).toEqual({ co_change: 3, used_together: 0, work_order: 1 });
  });

  it('ranks a partner that changed with the file in 60 commits before partners of 2 commits', () => {
    // README.md (0) shares 60 of its commits with a/one.rs (1) at strength 0.75 and 2 of 2 with
    // each of the other five at 1.0. Ranks: 0.75 ln 61 = 3.083 against 1.0 ln 3 = 1.099, so
    // a/one.rs comes first, as the daemon ranks a file's partners.
    const co = (to: number, weight: number, commits: number): GraphLink => ({
      from: 0,
      to,
      kind: 'co_change',
      weight,
      commits,
      count: commits,
    });
    const scene = sevenFileScene([co(1, 0.75, 60), co(2, 1, 2), co(3, 1, 2), co(4, 1, 2)]);
    expect(strongestLinks(scene, 0, 25).shown.co_change.map((link) => link.path)).toEqual([
      'a/one.rs',
      'a/two.rs',
      'b/deep/leaf.rs',
      'b/mid.rs',
    ]);
  });
});

describe('detailFor', () => {
  const detail = {
    version: 1 as const,
    id: 4,
    path: 'b/mid.rs',
    facts: [],
    history: [],
    history_total: 0,
    history_reader: 'on' as const,
    symbols: [],
    symbols_total: 0,
    selected: [],
    told: [],
    editor_url: null,
  };

  it('shows the detail of the panel file only', () => {
    expect(detailFor({ status: 'ready', detail }, 4)).toEqual({ status: 'ready', detail });
    expect(detailFor({ status: 'ready', detail }, 5)).toEqual({ status: 'loading', id: 5 });
  });

  it('reads an error or a load for another file as this one still loading', () => {
    const error = { code: 'network', message: 'down' } as const;
    expect(detailFor({ status: 'error', id: 4, error }, 6)).toEqual({ status: 'loading', id: 6 });
    expect(detailFor({ status: 'error', id: 6, error }, 6)).toEqual({
      status: 'error',
      id: 6,
      error,
    });
    expect(detailFor({ status: 'idle' }, 6)).toEqual({ status: 'idle' });
  });
});

describe('distinctNames', () => {
  it('keeps base names that are unique, even when a path repeats', () => {
    expect(distinctNames(['src/a.rs', 'src/b.rs', 'src/a.rs'])).toEqual(['a.rs', 'b.rs', 'a.rs']);
  });

  it('adds the folder to names that collide, and the whole folder path when folders collide too', () => {
    expect(distinctNames(['src/orders/mod.rs', 'src/billing/mod.rs', 'lib.rs'])).toEqual([
      'mod.rs in orders',
      'mod.rs in billing',
      'lib.rs',
    ]);
    expect(distinctNames(['a/x/mod.rs', 'b/x/mod.rs', 'mod.rs'])).toEqual([
      'mod.rs in a/x',
      'mod.rs in b/x',
      'mod.rs in the root',
    ]);
  });
});
