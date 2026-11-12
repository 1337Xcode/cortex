// The token table's rules, with literal expected values: every size on the 4 px grid, the spacing
// steps growing by about the golden ratio, the type steps and their grid line heights, the colour
// hues (one accent, its split complement for the status hues at matched lightness and chroma),
// and WCAG AA contrast for every text and mark pair in both themes. The contrast ratio is computed
// here with the WCAG 2.x formulas from the sRGB channels, which the OKLCH conversion gives (that
// conversion is checked against reference colours in oklch.test.ts and against the browser in
// e2e/colors.spec.ts).
import { describe, expect, it } from 'vitest';
import { LINK_KINDS } from '../api/graph';
import { isInSrgb, rgbaOf, type Oklch } from './oklch';
import {
  ACCENT_HUE,
  colorRoles,
  DARK_COLORS,
  GOLDEN_RATIO,
  GRID_PX,
  LIGHT_COLORS,
  LINK_DASH_PX,
  NEUTRAL,
  OPACITY,
  SIGNAL,
  SIZE,
  SPACE,
  STATUS_HUE,
  TYPE,
  type ThemeColors,
} from './tokens';

/** WCAG relative luminance of 0 to 255 sRGB channels. */
function luminance([r, g, b]: readonly number[]): number {
  const linear = (value: number): number => {
    const unit = value / 255;
    return unit <= 0.04045 ? unit / 12.92 : ((unit + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * linear(r ?? 0) + 0.7152 * linear(g ?? 0) + 0.0722 * linear(b ?? 0);
}

/** WCAG contrast ratio between two colours. */
function contrast(a: Oklch, b: Oklch): number {
  const [light, dark] = [luminance(rgbaOf(a)), luminance(rgbaOf(b))].sort((x, y) => y - x);
  return ((light ?? 0) + 0.05) / ((dark ?? 0) + 0.05);
}

describe('the grid and the scales', () => {
  it('spaces by 4, 8, 12, 20, 32 and 52 px, every step on the 4 px grid', () => {
    expect(GRID_PX).toBe(4);
    expect(SPACE).toEqual({ xs: 4, sm: 8, md: 12, lg: 20, xl: 32, xxl: 52 });
    for (const value of Object.values(SPACE)) expect(value % GRID_PX, String(value)).toBe(0);
  });

  it('grows each spacing step from 8 on by the golden ratio, within 0.15', () => {
    // 12 / 8 = 1.5, 20 / 12 = 1.667, 32 / 20 = 1.6, 52 / 32 = 1.625; the ratio is 1.618.
    const steps = Object.values(SPACE);
    for (let i = steps.indexOf(8) + 1; i < steps.length; i += 1) {
      const ratio = (steps[i] ?? 0) / (steps[i - 1] ?? 1);
      expect(Math.abs(ratio - GOLDEN_RATIO), `${steps[i - 1]} to ${steps[i]}`).toBeLessThanOrEqual(
        0.15,
      );
    }
  });

  it('sets type at 12, 13, 16 and 21 px on line heights of 16, 20, 24 and 32', () => {
    expect(Object.values(TYPE).map((step) => [step.sizePx, step.lineHeightPx])).toEqual([
      [12, 16],
      [13, 20],
      [16, 24],
      [21, 32],
    ]);
    for (const step of Object.values(TYPE)) expect(step.lineHeightPx % GRID_PX).toBe(0);
    // A major step by the ratio, a minor one by its square root: 13 x 1.618 = 21.03, 13 x 1.272 = 16.5.
    expect(Math.abs(TYPE.body.sizePx * GOLDEN_RATIO - TYPE.title.sizePx)).toBeLessThan(1);
    expect(Math.abs(TYPE.body.sizePx * Math.sqrt(GOLDEN_RATIO) - TYPE.heading.sizePx)).toBeLessThan(
      1,
    );
  });

  it('sizes every component on the grid', () => {
    expect(SIZE).toEqual({
      control: 32,
      controlCompact: 24,
      row: 28,
      icon: 16,
      iconTitle: 20,
      keyHint: 16,
      panel: 340,
      overview: 300,
      legend: 380,
      zoomValue: 48,
      intro: 380,
      radius: 8,
      radiusSmall: 4,
      swatchWidth: 28,
      swatchHeight: 16,
    });
    for (const [name, value] of Object.entries(SIZE)) expect(value % GRID_PX, name).toBe(0);
  });
});

describe('the hues', () => {
  it('puts green and amber 35 degrees either side of the accent violet complement', () => {
    // Violet 290; its complement 110; recent 110 + 35 = 145, suspect 110 - 35 = 75.
    expect([ACCENT_HUE, STATUS_HUE.recent, STATUS_HUE.suspect]).toEqual([290, 145, 75]);
  });

  it.each([
    ['light', LIGHT_COLORS, SIGNAL.light],
    ['dark', DARK_COLORS, SIGNAL.dark],
  ] as const)(
    '%s: gives the accent and every status the same lightness and chroma',
    (_, colors, signal) => {
      const signals = [colors.accent, colors.danger, colors.canvas.recent, colors.canvas.suspect];
      for (const color of signals) expect([color.l, color.c]).toEqual([signal.l, signal.c]);
    },
  );

  it('keeps twelve neutral steps per theme, darkest to lightest in dark and the other way in light', () => {
    for (const ramp of [NEUTRAL.dark, NEUTRAL.light]) expect(ramp).toHaveLength(12);
    const lightness = (ramp: readonly Oklch[]): number[] => ramp.map((color) => color.l);
    expect(lightness(NEUTRAL.dark)).toEqual([...lightness(NEUTRAL.dark)].sort((a, b) => a - b));
    expect(lightness(NEUTRAL.light)).toEqual([...lightness(NEUTRAL.light)].sort((a, b) => b - a));
  });
});

const THEME_CASES: [string, ThemeColors][] = [
  ['light', LIGHT_COLORS],
  ['dark', DARK_COLORS],
];

describe('contrast helper', () => {
  it('matches the known extremes: black on white is 21:1, equal colours 1:1', () => {
    const white = { l: 1, c: 0, h: 0 };
    const black = { l: 0, c: 0, h: 0 };
    expect(contrast(black, white)).toBeCloseTo(21, 6);
    expect(contrast(white, white)).toBeCloseTo(1, 6);
  });
});

describe.each(THEME_CASES)('%s theme', (_name, colors) => {
  const { canvas } = colors;

  it('draws every colour inside the sRGB gamut', () => {
    for (const [role, color] of colorRoles(colors)) expect(isInSrgb(color), role).toBe(true);
  });

  it.each([
    ['text on the background', colors.text, colors.background],
    ['text on a surface', colors.text, colors.surface],
    ['text on a raised surface', colors.text, colors.raised],
    ['muted text on the background', colors.textMuted, colors.background],
    ['muted text on a surface', colors.textMuted, colors.surface],
    ['muted text on a raised surface', colors.textMuted, colors.raised],
    ['accent text on a surface', colors.accent, colors.surface],
    ['accent text on the background', colors.accent, colors.background],
    ['text on the accent', colors.onAccent, colors.accent],
    ['an error on a surface', colors.danger, colors.surface],
    ['the suspect word on a surface', canvas.suspect, colors.surface],
    ['a tooltip, the ground on the text colour', colors.background, colors.text],
    ['a canvas label on the background', canvas.label, colors.background],
  ])('reads %s at 4.5:1 or more', (_label, text, ground) => {
    expect(contrast(text, ground)).toBeGreaterThanOrEqual(4.5);
  });

  it.each([
    ['a file', canvas.file],
    ['a never-read file', canvas.unread],
    ['a folder', canvas.hub],
    ['a recent file', canvas.recent],
    ['a suspect file', canvas.suspect],
    ['a learned link and the selection', colors.accent],
  ])('draws %s at 3:1 or more against the background', (_label, mark) => {
    expect(contrast(mark, colors.background)).toBeGreaterThanOrEqual(3);
  });

  it('draws the focus ring at 3:1 or more on a surface', () => {
    expect(contrast(colors.accent, colors.surface)).toBeGreaterThanOrEqual(3);
  });

  it('dims never-read files below read ones', () => {
    expect(contrast(canvas.unread, colors.background)).toBeLessThan(
      contrast(canvas.file, colors.background),
    );
  });

  it.each([
    ['the lit file of the Fog view', canvas.lit],
    ['a fact badge', canvas.badge],
    ...canvas.scale.map((color, i): [string, typeof color] => [
      `step ${i + 1} of the scale`,
      color,
    ]),
  ])('draws %s at 3:1 or more against the background', (_label, mark) => {
    expect(contrast(mark, colors.background)).toBeGreaterThanOrEqual(3);
  });

  it.each(Object.entries(colors.agents))(
    "writes the %s agent's marker name at 4.5:1 or more on the background",
    (_family, color) => {
      expect(contrast(color, colors.background)).toBeGreaterThanOrEqual(4.5);
    },
  );

  it('steps the scale away from the ground at matched hue and chroma, and lights the Fog above never read', () => {
    const steps = canvas.scale.map((color) => contrast(color, colors.background));
    expect(steps).toEqual([...steps].sort((x, y) => x - y));
    expect(new Set(steps).size).toBe(4);
    for (const color of canvas.scale)
      expect([color.c, color.h]).toEqual([colors.accent.c, ACCENT_HUE]);
    expect(contrast(canvas.lit, colors.background)).toBeGreaterThan(
      contrast(canvas.unread, colors.background),
    );
  });

  it('keeps the grid fainter than every drawn line', () => {
    const grid = contrast(canvas.gridDot, colors.background);
    expect(grid).toBeLessThan(contrast(canvas.hubLink, colors.background));
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
