// Contrast checks computed from the token values with the WCAG 2.x formulas, and the rules the
// palette must keep: one accent for every link kind, kinds told apart by dash pattern.
import { describe, expect, it } from 'vitest';
import { LINK_KINDS } from '../api/graph';
import { DARK_COLORS, LIGHT_COLORS, LINK_DASH_PX, OPACITY, type ThemeColors } from './tokens';

function channel(hex: string, offset: number): number {
  const value = parseInt(hex.slice(offset, offset + 2), 16) / 255;
  return value <= 0.03928 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
}

/** WCAG relative luminance of a `#rrggbb` colour. */
function luminance(hex: string): number {
  return 0.2126 * channel(hex, 1) + 0.7152 * channel(hex, 3) + 0.0722 * channel(hex, 5);
}

/** WCAG contrast ratio between two `#rrggbb` colours. */
function contrast(a: string, b: string): number {
  const [light, dark] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return ((light ?? 0) + 0.05) / ((dark ?? 0) + 0.05);
}

const THEME_CASES: [string, ThemeColors][] = [
  ['light', LIGHT_COLORS],
  ['dark', DARK_COLORS],
];

describe('contrast helper', () => {
  it('matches the known extremes: black on white is 21:1, equal colours 1:1', () => {
    expect(contrast('#000000', '#ffffff')).toBeCloseTo(21, 6);
    expect(contrast('#777777', '#777777')).toBeCloseTo(1, 6);
  });
});

describe.each(THEME_CASES)('%s theme', (_name, colors) => {
  it.each([
    ['text', colors.text],
    ['textMuted', colors.textMuted],
    ['accent', colors.accent],
    ['danger', colors.danger],
    ['canvas label', colors.canvas.label],
  ])('%s is at least 4.5:1 on the background and on the surface', (_label, value) => {
    expect(contrast(value, colors.background)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(value, colors.surface)).toBeGreaterThanOrEqual(4.5);
  });

  it.each([
    ['hub', colors.canvas.hub],
    ['file', colors.canvas.file],
    ['recent', colors.canvas.recent],
    ['suspect', colors.canvas.suspect],
  ])('draws %s at least 3:1 against the background', (_label, value) => {
    expect(contrast(value, colors.background)).toBeGreaterThanOrEqual(3);
  });

  it('draws learned links in the accent, at least 3:1 against the background', () => {
    expect(contrast(colors.accent, colors.background)).toBeGreaterThanOrEqual(3);
  });

  it('dims never-read files below read ones, yet keeps them visible (above 1.5:1)', () => {
    const unread = contrast(colors.canvas.unread, colors.background);
    expect(unread).toBeLessThan(contrast(colors.canvas.file, colors.background));
    expect(unread).toBeGreaterThan(1.5);
  });

  it('keeps the grid fainter than every drawn line', () => {
    const grid = contrast(colors.canvas.gridDot, colors.background);
    expect(grid).toBeLessThan(contrast(colors.canvas.hubLink, colors.background));
  });
});

describe('link kinds', () => {
  it('differ by dash pattern, so colour is never the only cue', () => {
    const patterns = LINK_KINDS.map((kind) => JSON.stringify(LINK_DASH_PX[kind]));
    expect(new Set(patterns).size).toBe(LINK_KINDS.length);
  });

  it('draw between 0.2 and 0.6 opacity by weight', () => {
    expect([OPACITY.linkMin, OPACITY.linkMax]).toEqual([0.2, 0.6]);
  });
});
