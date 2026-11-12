// Tests for the shortcut table, the search box's state rules and the text formatting the chrome
// uses.
import { describe, expect, it } from 'vitest';
import { sevenFileScene } from '../map/testScene';
import { countOf, formatCount, formatDate, formatPercent } from '../text/format';
import { shortcutFor, type KeyInput } from './shortcuts';
import { checkResults, stateForText } from './useSearch';

function key(name: string, change: Partial<KeyInput> = {}): KeyInput {
  return { key: name, ctrlKey: false, altKey: false, metaKey: false, isInField: false, ...change };
}

describe('shortcutFor', () => {
  it('maps the keys the controls show', () => {
    expect(
      ['/', 'z', 'C', '+', '-', 'ArrowUp', 'Escape'].map((name) => shortcutFor(key(name))),
    ).toEqual(['focusSearch', 'fit', 'center', 'zoomIn', 'zoomOut', 'panUp', 'clear']);
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

  it('writes a date for every time a date can hold and says so past it, never throwing', () => {
    expect(formatDate(0)).toBe('1970-01-01');
    expect(formatDate(8.64e15)).toBe('275760-09-13');
    expect(formatDate(8.64e15 + 1)).toBe('unknown date');
    expect(formatDate(Number.NaN)).toBe('unknown date');
  });
});
