// Tests for the level-of-detail helper on the seven-file tree, whose covering radii are 120 (root),
// 50 (b) and 15 (a, c and b/deep). A hub is open while radius * 2^zoom is at least 24 pixels.
import { describe, expect, it } from 'vitest';
import { HubState, levelOfDetail, pixelsPerUnit } from './lod';
import { layoutPaths } from './radial';

const layout = layoutPaths([
  'README.md',
  'a/one.rs',
  'a/two.rs',
  'b/deep/leaf.rs',
  'b/mid.rs',
  'b/zed.rs',
  'c/only.rs',
]);
// Hub order: root, a, b, b/deep, c.
const { Open, Collapsed, Hidden } = HubState;

describe('levelOfDetail', () => {
  it('opens every hub at zoom 1 (15 * 2 = 30 pixels is the smallest)', () => {
    expect(Array.from(levelOfDetail(layout, 1).state)).toEqual([Open, Open, Open, Open, Open]);
  });

  it('collapses the 15-unit hubs at zoom 0 and keeps the root and b open', () => {
    // 120 and 50 pixels are at least 24; 15 is not.
    expect(Array.from(levelOfDetail(layout, 0).state)).toEqual([
      Open,
      Collapsed,
      Open,
      Collapsed,
      Collapsed,
    ]);
  });

  it('collapses b and hides b/deep at zoom -2 (50 * 0.25 = 12.5 pixels)', () => {
    expect(Array.from(levelOfDetail(layout, -2).state)).toEqual([
      Open,
      Collapsed,
      Collapsed,
      Hidden,
      Collapsed,
    ]);
  });

  it('collapses the root and hides everything else at zoom -4 (120 / 16 = 7.5 pixels)', () => {
    expect(Array.from(levelOfDetail(layout, -4).state)).toEqual([
      Collapsed,
      Hidden,
      Hidden,
      Hidden,
      Hidden,
    ]);
  });
});

describe('pixels per unit', () => {
  it('doubles the scale at each zoom step', () => {
    expect(pixelsPerUnit(0)).toBe(1);
    expect(pixelsPerUnit(3)).toBe(8);
    expect(pixelsPerUnit(-1)).toBe(0.5);
  });
});
