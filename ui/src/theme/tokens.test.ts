// Contrast checks computed from the token values with the WCAG 2.x formulas.
import { describe, expect, it } from 'vitest';
import { LINK_KINDS } from '../api/graph';
import { DARK_COLORS, LIGHT_COLORS, type ThemeColors } from './tokens';

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
  ])('%s is at least 4.5:1 on the background and on the surface', (_label, value) => {
    expect(contrast(value, colors.background)).toBeGreaterThanOrEqual(4.5);
    expect(contrast(value, colors.surface)).toBeGreaterThanOrEqual(4.5);
  });

  it('draws every link kind at least 3:1 against the background', () => {
    for (const kind of LINK_KINDS) {
      expect(contrast(colors.link[kind], colors.background), kind).toBeGreaterThanOrEqual(3);
    }
  });

  it('gives the three link kinds three different colours', () => {
    expect(new Set(LINK_KINDS.map((kind) => colors.link[kind])).size).toBe(LINK_KINDS.length);
  });
});
