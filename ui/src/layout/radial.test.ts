// Tests for the radial layout. Expected values for the small tree are worked out by hand in the
// comment below; the other tests check properties (determinism, stability, spacing, speed).
import { describe, expect, it } from 'vitest';
import { HUB_PADDING, RING_SPACING } from './constants';
import { layoutPaths, type MapLayout } from './radial';
import { generatePaths, shuffled } from './testPaths';

/*
 * The seven-file tree, in byte order (ids 0 to 6):
 *
 *   README.md  a/one.rs  a/two.rs  b/deep/leaf.rs  b/mid.rs  b/zed.rs  c/only.rs
 *
 * Hubs in pre-order, children by name: 0 root, 1 a, 2 b, 3 b/deep, 4 c.
 * Constants: ring spacing R = 10, subtree gap G = 10, hub padding P = 5, minimum sector share 0.25.
 * Ring k has 6k slots, slot s at angle 2*pi*s/(6k), at distance k*R from its hub.
 *
 * Covering radii (own rings one ring = 10; padding 5):
 *   a, c, b/deep: one ring, no children         -> 10 + 5 = 15
 *   b: own 10; one child b/deep, sector 2*pi.   -> child distance = max(10 + 15 + 10, (15 + 10)/sin(pi/2))
 *      = max(35, 25) = 35; reach 35 + 15 = 50; radius = 50 + 5 = 55
 *   root: own 10; children a (2 files), b (3), c (1), total 6; floor = 0.25 * 2*pi / 3 = pi/6,
 *      spare = 2*pi - 3*pi/6 = 3*pi/2:
 *        sector a = pi/6 + 3*pi/2 * 2/6 = 2*pi/3  (120 degrees)
 *        sector b = pi/6 + 3*pi/2 * 3/6 = 11*pi/12 (165 degrees)
 *        sector c = pi/6 + 3*pi/2 * 1/6 = 5*pi/12 (75 degrees)
 *      distance = max(10 + 55 + 10, (15 + 10)/sin 60, (55 + 10)/sin 82.5, (15 + 10)/sin 37.5)
 *               = max(75, 28.87, 65.56, 41.07) = 75
 *      reach = 75 + 55 = 130; radius = 130 + 5 = 135.
 *
 * Sector centres from angle 0: a 60 degrees; b 120 + 82.5 = 202.5; c 285 + 37.5 = 322.5.
 *   a  = 75 * (cos 60, sin 60)      = (37.5, 64.951905284)
 *   b  = 75 * (cos 202.5, sin 202.5) = (-69.290964938, -28.701257427)
 *   c  = 75 * (cos 322.5, sin 322.5) = (59.501500522, -45.657107176)
 * b has one sub-directory, whose sector is the full circle, so its centre is at 180 degrees,
 * 35 away: b/deep = b + (-35, 0) = (-104.290964938, -28.701257427).
 *
 * Files: slot 0 of ring 1 is (+10, 0) from the hub; slot 1 is 10 * (cos 60, sin 60) = (5, 8.660254038).
 *   README.md  root slot 0 = (10, 0)
 *   a/one.rs   a + (10, 0)     = (47.5, 64.951905284)
 *   a/two.rs   a + (5, 8.66...) = (42.5, 73.612159322)
 *   b/deep/leaf.rs  deep + (10, 0) = (-94.290964938, -28.701257427)
 *   b/mid.rs   b + (10, 0)     = (-59.290964938, -28.701257427)
 *   b/zed.rs   b + (5, 8.66...) = (-64.290964938, -20.041003390)
 *   c/only.rs  c + (10, 0)     = (69.501500522, -45.657107176)
 *
 * Positions are stored as float32, which has a step of about 7.6e-6 near 65, so each expected
 * value is rounded to float32 before the comparison and the two must agree to 1e-6.
 */
const SEVEN_FILES = [
  'README.md',
  'a/one.rs',
  'a/two.rs',
  'b/deep/leaf.rs',
  'b/mid.rs',
  'b/zed.rs',
  'c/only.rs',
];

function expectFloat32(actual: number | undefined, expected: number): void {
  expect(actual).toBeCloseTo(Math.fround(expected), 6);
}

/**
 * Compares two offsets that were each computed as a difference of float32 positions near 100,
 * which carries about 4e-6 of rounding, so the tolerance is 1e-4.
 */
function expectClose(actual: number | undefined, expected: number): void {
  expect(actual).toBeCloseTo(expected, 4);
}

function bytesOf(layout: MapLayout): Uint8Array[] {
  const arrays = [
    layout.hubX,
    layout.hubY,
    layout.hubParent,
    layout.hubRadius,
    layout.hubFileCount,
    layout.fileX,
    layout.fileY,
    layout.fileHub,
  ];
  return arrays.map((array) => new Uint8Array(array.buffer, array.byteOffset, array.byteLength));
}

function expectByteEqual(a: MapLayout, b: MapLayout): void {
  expect(bytesOf(a)).toEqual(bytesOf(b));
  expect(a.hubPath).toEqual(b.hubPath);
}

/** Absolute position by path, for comparing layouts whose file ids differ. */
function positionsByPath(
  paths: readonly string[],
  layout: MapLayout,
): Map<string, [number, number]> {
  const sorted = [...paths].sort();
  return new Map(sorted.map((path, i) => [path, [layout.fileX[i] ?? NaN, layout.fileY[i] ?? NaN]]));
}

/** Position relative to the file's own hub, by path. */
function offsetsByPath(paths: readonly string[], layout: MapLayout): Map<string, [number, number]> {
  const sorted = [...paths].sort();
  return new Map(
    sorted.map((path, i) => {
      const hub = layout.fileHub[i] ?? 0;
      return [
        path,
        [
          (layout.fileX[i] ?? NaN) - (layout.hubX[hub] ?? NaN),
          (layout.fileY[i] ?? NaN) - (layout.hubY[hub] ?? NaN),
        ],
      ];
    }),
  );
}

describe('layoutPaths on the seven-file tree', () => {
  const layout = layoutPaths(SEVEN_FILES);

  it('numbers hubs in pre-order with their parents, paths, file counts and covering radii', () => {
    expect(layout.hubPath).toEqual(['', 'a', 'b', 'b/deep', 'c']);
    expect(Array.from(layout.hubParent)).toEqual([-1, 0, 0, 2, 0]);
    expect(Array.from(layout.hubFileCount)).toEqual([7, 2, 3, 1, 1]);
    expect(Array.from(layout.hubRadius)).toEqual([135, 15, 55, 15, 15]);
    expect(Array.from(layout.fileHub)).toEqual([0, 1, 1, 3, 2, 2, 4]);
  });

  it('puts the hubs where the worked example puts them', () => {
    const expected: [number, number][] = [
      [0, 0],
      [37.5, 64.951905284],
      [-69.290964938, -28.701257427],
      [-104.290964938, -28.701257427],
      [59.501500522, -45.657107176],
    ];
    for (const [h, [x, y]] of expected.entries()) {
      expectFloat32(layout.hubX[h], x);
      expectFloat32(layout.hubY[h], y);
    }
  });

  it('puts the files where the worked example puts them', () => {
    const expected: [number, number][] = [
      [10, 0],
      [47.5, 64.951905284],
      [42.5, 73.612159322],
      [-94.290964938, -28.701257427],
      [-59.290964938, -28.701257427],
      [-64.290964938, -20.04100339],
      [69.501500522, -45.657107176],
    ];
    for (const [f, [x, y]] of expected.entries()) {
      expectFloat32(layout.fileX[f], x);
      expectFloat32(layout.fileY[f], y);
    }
  });
});

describe('layoutPaths determinism and stability', () => {
  const paths = generatePaths(2000, 6);
  const base = layoutPaths(paths);

  it('gives byte-equal arrays for shuffled input', () => {
    expectByteEqual(layoutPaths(shuffled(paths)), base);
    expectByteEqual(layoutPaths(shuffled(SEVEN_FILES)), layoutPaths(SEVEN_FILES));
  });

  it('moves nothing but the files after a new root file in the root directory', () => {
    // B.md sorts before README.md: README.md moves from slot 0 to slot 1 and nothing else moves,
    // because the root's ring count and its sub-directories' counts are unchanged.
    const before = layoutPaths(SEVEN_FILES);
    const withNew = [...SEVEN_FILES, 'B.md'];
    const after = layoutPaths(withNew);
    const was = positionsByPath(SEVEN_FILES, before);
    const now = positionsByPath(withNew, after);
    for (const [path, position] of was) {
      if (path === 'README.md') continue;
      expect(now.get(path), path).toEqual(position);
    }
    expectFloat32(now.get('README.md')?.[0], 5);
    expectFloat32(now.get('README.md')?.[1], 8.660254038);
    expectFloat32(now.get('B.md')?.[0], 10);
    expect(Array.from(after.hubX.slice(0, 5))).toEqual(Array.from(before.hubX));
    expect(Array.from(after.hubY.slice(0, 5))).toEqual(Array.from(before.hubY));
  });

  it('keeps unchanged sub-trees rigid, but moves their hubs, when a file joins a sibling directory', () => {
    // a/three.rs sorts between a/one.rs and a/two.rs: a/two.rs moves from slot 1 to slot 2.
    // Sector sizes at the root are shares of the root's total, so a's count 2 -> 3 resizes every
    // sibling's sector: hubs b and c cannot keep their absolute positions. What stays fixed is
    // each unchanged sub-tree's shape: every file and sub-hub keeps its offset from its own hub.
    const before = layoutPaths(SEVEN_FILES);
    const withNew = [...SEVEN_FILES, 'a/three.rs'];
    const after = layoutPaths(withNew);
    const was = offsetsByPath(SEVEN_FILES, before);
    const now = offsetsByPath(withNew, after);
    for (const [path, offset] of was) {
      if (path === 'a/two.rs') continue;
      expectClose(now.get(path)?.[0], offset[0]);
      expectClose(now.get(path)?.[1], offset[1]);
    }
    // Slot 2 of ring 1 is at 2 * 2*pi/6 = 120 degrees: (-5, 8.660254038).
    expectClose(now.get('a/two.rs')?.[0], -5);
    expectClose(now.get('a/two.rs')?.[1], 8.660254038);
    // b's hub moved because its sector was resized, and b/deep moved with it, by the same amount.
    const bMoved = Math.hypot(
      (after.hubX[2] ?? 0) - (before.hubX[2] ?? 0),
      (after.hubY[2] ?? 0) - (before.hubY[2] ?? 0),
    );
    expect(bMoved).toBeGreaterThan(1);
    expectClose((after.hubX[3] ?? 0) - (after.hubX[2] ?? 0), -35);
    expectClose((after.hubY[3] ?? 0) - (after.hubY[2] ?? 0), 0);
  });
});

/** Asserts each sub-tree's circle lies inside its parent's and apart from its siblings'. */
function expectSubtreesApart(layout: MapLayout): void {
  const hubCount = layout.hubParent.length;
  const hubDistance = (a: number, b: number): number =>
    Math.hypot(
      (layout.hubX[a] ?? 0) - (layout.hubX[b] ?? 0),
      (layout.hubY[a] ?? 0) - (layout.hubY[b] ?? 0),
    );
  for (let a = 1; a < hubCount; a += 1) {
    const parent = layout.hubParent[a] ?? 0;
    expect(hubDistance(a, parent) + (layout.hubRadius[a] ?? 0)).toBeLessThanOrEqual(
      (layout.hubRadius[parent] ?? 0) - HUB_PADDING + 1e-3,
    );
    for (let b = a + 1; b < hubCount; b += 1) {
      if (layout.hubParent[b] !== parent) continue;
      expect(hubDistance(a, b)).toBeGreaterThanOrEqual(
        (layout.hubRadius[a] ?? 0) + (layout.hubRadius[b] ?? 0) - 1e-3,
      );
    }
  }
}

describe('layoutPaths geometry', () => {
  it('keeps every pair of 2,000 files at least one ring spacing apart', () => {
    const layout = layoutPaths(generatePaths(2000, 6));
    let closest = Infinity;
    for (let i = 0; i < 2000; i += 1) {
      for (let j = i + 1; j < 2000; j += 1) {
        const dx = (layout.fileX[i] ?? 0) - (layout.fileX[j] ?? 0);
        const dy = (layout.fileY[i] ?? 0) - (layout.fileY[j] ?? 0);
        closest = Math.min(closest, Math.hypot(dx, dy));
      }
    }
    expect(closest).toBeGreaterThanOrEqual(RING_SPACING - 1e-3);
  });

  it('keeps sibling sub-tree circles apart and inside their parent circle', () => {
    expectSubtreesApart(layoutPaths(generatePaths(2000, 6)));
  });

  it('lays out an empty repository as a lone root hub', () => {
    const layout = layoutPaths([]);
    expect(layout.hubPath).toEqual(['']);
    expect(layout.fileX.length).toBe(0);
    expect(Array.from(layout.hubParent)).toEqual([-1]);
  });

  it('lays out 100,000 files with every position finite', () => {
    // Its speed is measured in the browser with the map's frame-rate check, not here: a
    // wall-clock bound in the default suite fails under load.
    const layout = layoutPaths(generatePaths(100_000, 12));
    expect(layout.fileX.length).toBe(100_000);
    expect(layout.fileX.every(Number.isFinite)).toBe(true);
    expect(layout.fileY.every(Number.isFinite)).toBe(true);
  });

  it('keeps a wide, lopsided fan-out apart', () => {
    // One directory of 500 files beside eight of one file each: the big one's sector is wider
    // than a half turn (0.25 of an even share as a floor, the rest by count: about 4.8 rad).
    const big = Array.from({ length: 500 }, (_, i) => `big/f${String(i).padStart(3, '0')}.rs`);
    const small = Array.from({ length: 8 }, (_, i) => `d${i}/x.rs`);
    expectSubtreesApart(layoutPaths([...big, ...small]));
  });
});
