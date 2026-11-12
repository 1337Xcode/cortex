// Tests for the scene, the folder encoders and the focus neighbourhood on the seven-file tree:
// hub-to-parent lines skip hidden hubs, collapsed nodes are sized by their sub-tree's reach, file
// discs appear only while files are not shown one by one.
import { describe, expect, it } from 'vitest';
import { levelOfDetail } from '../layout/lod';
import type { FrameStyle } from './frameStyle';
import { neighbourhoodOf } from './highlight';
import {
  collapsedRadius,
  encodeFileDiscs,
  encodeHubLinks,
  encodeSpokes,
  focusHubs,
} from './hubEncoding';
import { RING_SPACING } from '../layout/constants';
import { layoutPaths } from '../layout/radial';
import { generatePaths } from '../layout/testPaths';
import { CANVAS_SIZE, DARK_COLORS } from '../theme/tokens';
import { suspectRingRadius } from './fileMarks';
import { buildAdjacency, buildScene, fileRadiusFor, ownRingsRadius } from './scene';
import { sevenFileScene, sevenFileSnapshot, testFile, testLink } from './testScene';
import { computeVisibility } from './visibility';

const style: FrameStyle = {
  colors: DARK_COLORS,
  nowMs: 0,
  neighbourhood: null,
  selected: null,
  minStrength: 0.25,
};

describe('scene', () => {
  it('sizes files by the square root of their lines, clamped to 1.2 to 2.8 world units', () => {
    expect(fileRadiusFor(0)).toBeCloseTo(1.2, 9);
    expect(fileRadiusFor(100)).toBeCloseTo(1.9, 9);
    expect(fileRadiusFor(10_000)).toBe(2.8);
  });

  it('never lets two file circles touch, with the thick outline, wherever files are drawn', () => {
    // The closest two files of a 2,000-file layout are one ring spacing apart (the layout's own
    // property); two of the largest circles, suspect rings and outlines included, must leave a gap at that distance
    // at every zoom from the one where single files appear (8 px between rings) to the closest.
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
    const firstScale = CANVAS_SIZE.fileMinRingSpacingPx / RING_SPACING;
    for (let scale = firstScale; scale <= 32; scale *= 1.1) {
      const outline = CANVAS_SIZE.fileStrokeFocusedPx / 2;
      // The largest file with its suspect ring, the outermost mark.
      const outer = suspectRingRadius(CANVAS_SIZE.fileRadiusMax);
      const radiusPx = Math.max(outer * scale, CANVAS_SIZE.fileRadiusMinPx);
      expect(2 * (radiusPx + outline), `at ${scale} px a unit`).toBeLessThan(closest * scale);
    }
  });

  it('gives own-file rings of 6, 12, 18 slots: 1 to 6 files one ring, 7 to 18 two, 19 three', () => {
    expect([0, 1, 6, 7, 18, 19].map(ownRingsRadius)).toEqual([0, 10, 10, 20, 20, 30]);
  });

  it('indexes the links of each file as one run, in link order', () => {
    const links = [
      testLink(0, 1, 'co_change', 1),
      testLink(0, 6, 'work_order', 1),
      testLink(1, 6, 'work_order', 1),
    ];
    const { offsets, linkIds } = buildAdjacency(sevenFileSnapshot(links));
    // File 0: links 0, 1. File 1: links 0, 2. File 6: links 1, 2. The rest none.
    expect(Array.from(offsets)).toEqual([0, 2, 4, 4, 4, 4, 4, 6]);
    expect(Array.from(linkIds)).toEqual([0, 1, 0, 2, 1, 2]);
  });
});

describe('neighbourhoodOf', () => {
  const scene = sevenFileScene([
    testLink(0, 1, 'co_change', 0.9),
    testLink(0, 3, 'work_order', 0.6),
    testLink(0, 4, 'used_together', 0.2),
  ]);

  it('takes the links at or above the threshold, in id order', () => {
    expect(neighbourhoodOf(scene, 0, 0.25)?.neighbours).toEqual([1, 3]);
    expect(neighbourhoodOf(scene, 0, 0)?.neighbours).toEqual([1, 3, 4]);
    expect(neighbourhoodOf(scene, 4, 0.25)?.neighbours).toEqual([]);
  });

  it('is empty without a focus or with an id past the end', () => {
    expect(neighbourhoodOf(scene, null, 0)).toBeNull();
    expect(neighbourhoodOf(scene, 7, 0)).toBeNull();
  });

  it('marks the hubs of the focus and its neighbours', () => {
    const visibility = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), true);
    const neighbourhood = neighbourhoodOf(scene, 0, 0.25);
    // README in the root (0), a/one.rs in a (1), b/deep/leaf.rs in b/deep (3).
    expect(Array.from(focusHubs(scene, visibility, { ...style, neighbourhood }))).toEqual([
      1, 1, 0, 1, 0,
    ]);
  });
});

describe('folder encoders', () => {
  const scene = sevenFileScene();

  it('draws a line to the parent for every drawn hub, none for a hidden one', () => {
    // At zoom -2: a, b and c collapse (drawn), b/deep is hidden inside b.
    const visibility = computeVisibility(scene.layout, levelOfDetail(scene.layout, -2), true);
    const lines = encodeHubLinks(scene, visibility, style, new Uint8Array(5), [3, 4]);
    expect(Array.from(lines.ids)).toEqual([1, 2, 4]);
    expect(Array.from(lines.dashes.subarray(0, 2))).toEqual([3, 4]);
  });

  it('draws spokes in folders of up to two rings of files, none in bigger ones', () => {
    const visibility = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), true);
    expect(encodeSpokes(scene, visibility, style).ids.length).toBe(7);
    // 19 files need a third ring (6 + 12 = 18 fit on two).
    const paths = Array.from({ length: 19 }, (_, i) => `big/f${String(i).padStart(2, '0')}.rs`);
    const big = buildScene({
      version: 1,
      files: paths.map((path, id) => testFile(id, path)),
      links: [],
    });
    const bigVisibility = computeVisibility(big.layout, levelOfDetail(big.layout, 1), true);
    expect(encodeSpokes(big, bigVisibility, style).ids.length).toBe(0);
  });

  it('sizes a collapsed hub at 0.8 of its covering radius', () => {
    expect(collapsedRadius(scene, 2)).toBeCloseTo(40, 4);
    expect(collapsedRadius(scene, 1)).toBeCloseTo(12, 4);
  });

  it('draws own-file discs only while files are not shown one by one', () => {
    const lod = levelOfDetail(scene.layout, 1);
    const shown = computeVisibility(scene.layout, lod, true);
    const hidden = computeVisibility(scene.layout, lod, false);
    expect(encodeFileDiscs(scene, shown, style, new Uint8Array(5)).ids.length).toBe(0);
    // Each of the five hubs holds one or two files of its own: one ring, radius 10.
    const discs = encodeFileDiscs(scene, hidden, style, new Uint8Array(5));
    expect(Array.from(discs.ids)).toEqual([0, 1, 2, 3, 4]);
    expect(Array.from(discs.radii)).toEqual([10, 10, 10, 10, 10]);
  });
});
