// Tests for the shortcut table, the search box's state rules and the text formatting the chrome
// uses.
import { describe, expect, it } from 'vitest';
import { sevenFileSnapshot } from '../map/testScene';
import {
  countOf,
  formatCount,
  formatDate,
  formatMicros,
  formatPercent,
  formatSpan,
  formatUtcMoment,
} from '../text/format';
import { SECTION_KEYS, sectionFor, shortcutFor, viewIndexFor, type KeyInput } from './shortcuts';
import { firstListedId, foundResults, isAnswerAhead, stateForText } from './useSearch';

function key(name: string, change: Partial<KeyInput> = {}): KeyInput {
  return { key: name, ctrlKey: false, altKey: false, metaKey: false, isInField: false, ...change };
}

describe('shortcutFor', () => {
  it('maps the keys the controls show', () => {
    expect(
      ['/', 'z', 'C', '+', '-', 'ArrowUp', 'l', 'Escape'].map((name) => shortcutFor(key(name))),
    ).toEqual([
      'focusSearch',
      'fit',
      'center',
      'zoomIn',
      'zoomOut',
      'panUp',
      'toggleLegend',
      'clear',
    ]);
  });

  it('leaves keys typed into a field to the field, except Escape', () => {
    expect(shortcutFor(key('z', { isInField: true }))).toBeNull();
    expect(shortcutFor(key('/', { isInField: true }))).toBeNull();
    expect(shortcutFor(key('Escape', { isInField: true }))).toBe('clear');
  });

  it('leaves keys held with Ctrl, Alt or Meta to the browser', () => {
    expect(shortcutFor(key('z', { ctrlKey: true }))).toBeNull();
    expect(shortcutFor(key('+', { metaKey: true }))).toBeNull();
    expect(shortcutFor(key('q'))).toBeNull();
  });
});

describe('sectionFor', () => {
  it('opens each section by its letter, in either case', () => {
    expect(['a', 'W', 'r', 'f', 'h', 'T', 's'].map((name) => sectionFor(key(name)))).toEqual([
      'activity',
      'warnings',
      'receipts',
      'facts',
      'history',
      'told',
      'symbols',
    ]);
  });

  it('leaves other keys, keys typed into a field and keys held with a modifier alone', () => {
    expect(sectionFor(key('q'))).toBeNull();
    expect(sectionFor(key('h', { isInField: true }))).toBeNull();
    expect(sectionFor(key('h', { ctrlKey: true }))).toBeNull();
    expect(sectionFor(key('t', { metaKey: true }))).toBeNull();
  });

  it('shares no letter with another section or with the map', () => {
    const letters = Object.values(SECTION_KEYS);
    expect(new Set(letters).size).toBe(letters.length);
    for (const letter of letters) {
      expect(shortcutFor(key(letter))).toBeNull();
      expect(shortcutFor(key(letter.toUpperCase()))).toBeNull();
      expect(viewIndexFor(key(letter), 9)).toBeNull();
    }
  });
});

describe('viewIndexFor', () => {
  it('picks views 1 to 5 by their digit, and nothing past the count, in a field or with a modifier', () => {
    expect(['1', '3', '5', '6', '0', 'a'].map((name) => viewIndexFor(key(name), 5))).toEqual([
      0,
      2,
      4,
      null,
      null,
      null,
    ]);
    expect(viewIndexFor(key('2', { isInField: true }), 5)).toBeNull();
    expect(viewIndexFor(key('2', { altKey: true }), 5)).toBeNull();
  });
});

describe('search state', () => {
  it('starts idle for blank text, loading for a valid query, refused past 100 characters', () => {
    expect(stateForText('  ')).toEqual({ status: 'idle' });
    expect(stateForText('ord')).toEqual({ status: 'loading', query: 'ord' });
    expect(stateForText('x'.repeat(101))).toEqual({ status: 'too_long' });
  });

  it('finds each result by its path, never by an id the map holds another file at', () => {
    const { files } = sevenFileSnapshot();
    // c/only.rs is 6 on the map; the daemon names it 6, then 5 (its ids moved on), and names a
    // file the map does not hold yet at 7.
    const results = [
      { kind: 'file' as const, file: 6, path: 'c/only.rs', name: 'only.rs', line: null },
      { kind: 'symbol' as const, file: 5, path: 'c/only.rs', name: 'Only', line: 3 },
      { kind: 'file' as const, file: 7, path: 'c/new.rs', name: 'new.rs', line: null },
    ];
    expect(foundResults(files, results).map(({ named }) => named)).toEqual([
      { kind: 'listed', id: 6, path: 'c/only.rs' },
      { kind: 'listed', id: 6, path: 'c/only.rs' },
      { kind: 'pending', path: 'c/new.rs' },
    ]);
    expect(isAnswerAhead(files, results.slice(0, 1))).toBe(false);
    expect(isAnswerAhead(files, results.slice(1, 2))).toBe(true);
    expect(isAnswerAhead(files, results.slice(2))).toBe(true);
  });

  it('picks the first result the map holds, and none when it holds none', () => {
    const { files } = sevenFileSnapshot();
    const pending = {
      kind: 'file' as const,
      file: 7,
      path: 'c/new.rs',
      name: 'new.rs',
      line: null,
    };
    const listed = { kind: 'file' as const, file: 1, path: 'a/one.rs', name: 'one.rs', line: null };
    expect(firstListedId(foundResults(files, [pending, listed]))).toBe(1);
    expect(firstListedId(foundResults(files, [pending]))).toBeNull();
    // The id 1 is a/one.rs's; a result naming b/zed.rs there opens b/zed.rs (5), not a/one.rs.
    const moved = { ...listed, path: 'b/zed.rs' };
    expect(firstListedId(foundResults(files, [moved]))).toBe(5);
  });
});

describe('format', () => {
  it('groups thousands, picks the noun number, rounds percentages and writes UTC dates', () => {
    expect(formatCount(52310)).toBe('52,310');
    expect([countOf(1, 'file'), countOf(2, 'file'), countOf(0, 'commit')]).toEqual([
      '1 file',
      '2 files',
      '0 commits',
    ]);
    expect([formatPercent(312, 1204), formatPercent(0, 0)]).toEqual(['26%', '0%']);
    expect(formatDate(1762300000 * 1000)).toBe('2025-11-04');
  });

  it('never rounds a share that is neither none nor all to 0% or 100%', () => {
    // 2 of 1,595 is 0.125%; 1,594 of 1,595 is 99.94%; 5 of 1,000 is 0.5%, which is not below 1%.
    expect([
      formatPercent(2, 1595),
      formatPercent(0, 10),
      formatPercent(1594, 1595),
      formatPercent(10, 10),
      formatPercent(5, 1000),
      formatPercent(11, 1000),
    ]).toEqual(['under 1%', '0%', 'over 99%', '100%', 'under 1%', '1%']);
  });

  it('writes spans of time in their largest whole unit, and UTC times', () => {
    expect([999, 45_000, 150_000, 3 * 3_600_000 + 5, 4 * 86_400_000].map(formatSpan)).toEqual([
      'under a second',
      '45 seconds',
      '2 minutes',
      '3 hours',
      '4 days',
    ]);
    // 1762300000000 ms is 2025-11-04 23:46:40 UTC.
    expect(formatUtcMoment(1762300000000)).toBe('2025-11-04 23:46:40 UTC');
    expect([formatMicros(850), formatMicros(2400), formatMicros(1_234_567)]).toEqual([
      '850 µs',
      '2.4 ms',
      '1,234.6 ms',
    ]);
  });

  it('writes a date for every time a date can hold and says so past it, never throwing', () => {
    expect(formatDate(0)).toBe('1970-01-01');
    expect(formatDate(8.64e15)).toBe('275760-09-13');
    expect(formatDate(8.64e15 + 1)).toBe('unknown date');
    expect(formatDate(Number.NaN)).toBe('unknown date');
  });
});
