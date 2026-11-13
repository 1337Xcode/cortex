// Tests for the shortcut table, the search box's state rules and the text formatting the chrome
// uses.
import { describe, expect, it } from 'vitest';
import { sevenFileScene } from '../map/testScene';
import {
  countOf,
  formatCount,
  formatDate,
  formatMicros,
  formatPercent,
  formatSpan,
  formatUtcMoment,
} from '../text/format';
import { shortcutFor, viewIndexFor, type KeyInput } from './shortcuts';
import { checkResults, stateForText } from './useSearch';

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

  it('refuses an answer naming a file the map does not have', () => {
    const scene = sevenFileScene();
    const fine = [{ kind: 'file' as const, file: 6, name: 'only.rs', line: null }];
    expect(checkResults(scene, 'only', fine)).toEqual({
      status: 'ready',
      query: 'only',
      results: fine,
    });
    const unknown = [{ kind: 'file' as const, file: 7, name: 'ghost.rs', line: null }];
    const state = checkResults(scene, 'g', unknown);
    expect(state.status === 'error' ? state.error.code : state.status).toBe('bad_range');
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
