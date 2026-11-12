// The OKLCH conversion against reference colours whose OKLCH values CSS Color 4 and the OKLab
// definition give (sRGB red, green and blue, white and black), to within one step of 255.
import { describe, expect, it } from 'vitest';
import { cssOf, isInSrgb, oklch, rgbaOf } from './oklch';

function expectNear(actual: readonly number[], expected: readonly number[]): void {
  expect(actual.length).toBe(expected.length);
  for (const [index, value] of expected.entries()) {
    expect(Math.abs((actual[index] ?? Number.NaN) - value), `channel ${index}`).toBeLessThanOrEqual(
      1,
    );
  }
}

describe('rgbaOf', () => {
  it('converts the sRGB primaries, white and black from their OKLCH values', () => {
    expectNear(rgbaOf(oklch(0.62796, 0.25768, 29.2339)), [255, 0, 0, 255]);
    expectNear(rgbaOf(oklch(0.86644, 0.29483, 142.4953)), [0, 255, 0, 255]);
    expectNear(rgbaOf(oklch(0.45201, 0.31321, 264.052)), [0, 0, 255, 255]);
    expectNear(rgbaOf(oklch(1, 0, 0)), [255, 255, 255, 255]);
    expectNear(rgbaOf(oklch(0, 0, 0)), [0, 0, 0, 255]);
  });

  it('scales alpha from 0 to 1 onto 0 to 255, clamped', () => {
    expect(rgbaOf(oklch(1, 0, 0), 0.5)[3]).toBe(128);
    expect(rgbaOf(oklch(1, 0, 0), 2)[3]).toBe(255);
    expect(rgbaOf(oklch(1, 0, 0), -1)[3]).toBe(0);
  });
});

describe('isInSrgb', () => {
  it('accepts the primaries and refuses a chroma no sRGB colour reaches', () => {
    expect(isInSrgb(oklch(0.62796, 0.25768, 29.2339))).toBe(true);
    expect(isInSrgb(oklch(0.7, 0.4, 150))).toBe(false);
  });
});

describe('cssOf', () => {
  it('writes the colour as CSS reads it', () => {
    expect(cssOf(oklch(0.77, 0.12, 290))).toBe('oklch(0.77 0.12 290)');
  });
});
