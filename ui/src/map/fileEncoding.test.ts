// Tests for the file encoder: which mark a file shows, and the literal channels it writes.
import { describe, expect, it } from 'vitest';
import { levelOfDetail } from '../layout/lod';
import { DARK_COLORS, RECENT_WINDOW_MS } from '../theme/tokens';
import { fileMark, encodeFiles } from './fileEncoding';
import type { FrameStyle } from './frameStyle';
import { neighbourhoodOf } from './highlight';
import { encodeFileMarks } from './fileMarks';
import { buildScene } from './scene';
import { sevenFileScene, sevenFileSnapshot, testFile, testLink } from './testScene';
import { computeVisibility } from './visibility';

const NOW_MS = 1_762_900_000_000;

describe('fileMark', () => {
  it('shows a suspect fact above everything else', () => {
    const file = testFile(0, 'a.rs', { suspect: 1, touched_ms: NOW_MS, reads: 0 });
    expect(fileMark(file, NOW_MS, true)).toBe('suspect');
  });

  it('counts a touch exactly 24 hours old as recent, and one a millisecond older as not', () => {
    const edge = testFile(0, 'a.rs', { touched_ms: NOW_MS - RECENT_WINDOW_MS, reads: 1 });
    const older = testFile(0, 'a.rs', { touched_ms: NOW_MS - RECENT_WINDOW_MS - 1, reads: 1 });
    expect(fileMark(edge, NOW_MS, true)).toBe('recent');
    expect(fileMark(older, NOW_MS, true)).toBe('plain');
  });

  it('dims a never-read file only once some file has been read', () => {
    const unread = testFile(0, 'a.rs', { reads: 0 });
    expect(fileMark(unread, NOW_MS, true)).toBe('unread');
    expect(fileMark(unread, NOW_MS, false)).toBe('plain');
  });
});

describe('encodeFiles', () => {
  const scene = sevenFileScene([testLink(0, 4, 'co_change', 0.9)]);
  const visibility = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), true);
  const plain: FrameStyle = {
    colors: DARK_COLORS,
    nowMs: NOW_MS,
    neighbourhood: null,
    selected: 2,
    minStrength: 0.25,
  };

  it('writes positions from the layout and radii from the lines', () => {
    const files = encodeFiles(scene, visibility, plain);
    // README.md sits at (10, 0); 100 lines give 1.2 + 0.07 * 10 = 1.9 world units.
    expect([files.positions[0], files.positions[1]]).toEqual([10, 0]);
    expect(files.radii[0]).toBeCloseTo(1.9, 6);
  });

  it('outlines the selected file in the accent, thicker, and fills with the background', () => {
    const files = encodeFiles(scene, visibility, plain);
    // Dark accent #a98bfa, file #8e98a7, background #121418.
    expect(Array.from(files.lineColors.subarray(8, 12))).toEqual([169, 139, 250, 255]);
    expect(Array.from(files.lineColors.subarray(0, 4))).toEqual([142, 152, 167, 255]);
    expect(Array.from(files.fillColors.subarray(0, 4))).toEqual([18, 20, 24, 255]);
    expect(
      [files.lineWidths[2], files.lineWidths[0]].map((w) => Math.round((w ?? 0) * 10)),
    ).toEqual([22, 11]);
  });

  it('dims files outside the focus to 0.18 of their alpha (46 of 255)', () => {
    const neighbourhood = neighbourhoodOf(scene, 0, 0.25);
    const files = encodeFiles(scene, visibility, { ...plain, neighbourhood });
    // Focus README.md (0) and its neighbour b/mid.rs (4) stay full; a/one.rs (1) dims.
    expect([files.lineColors[3], files.lineColors[4 * 4 + 3], files.lineColors[1 * 4 + 3]]).toEqual(
      [255, 255, 46],
    );
  });
});

describe('second channel of the marks', () => {
  // README.md (0) has a suspect fact, a/one.rs (1) was touched an hour ago, a/two.rs (2) was
  // never read while b/mid.rs (4) was: one ring, one dot, one thin outline.
  const changes = new Map([
    [0, { suspect: 1, reads: 1 }],
    [1, { touched_ms: NOW_MS - 3_600_000, reads: 1 }],
    [4, { reads: 2 }],
  ]);
  const scene = buildScene(sevenFileSnapshot([], changes));
  const visibility = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), true);
  const style: FrameStyle = {
    colors: DARK_COLORS,
    nowMs: NOW_MS,
    neighbourhood: null,
    selected: null,
    minStrength: 0,
  };

  it('rings a suspect file 0.6 outside its circle and dots a recent one at 0.45 of its radius', () => {
    const marks = encodeFileMarks(scene, visibility, style);
    expect(Array.from(marks.ids)).toEqual([0, 1]);
    // 100 lines: radius 1.9, ring 2.5, dot 0.855.
    expect(marks.radii[0]).toBeCloseTo(2.5, 5);
    expect(marks.radii[1]).toBeCloseTo(0.855, 5);
    // Ring: suspect #fbbf24 outline, no fill. Dot: recent #4ade80 fill, no outline.
    expect(Array.from(marks.lineColors.subarray(0, 4))).toEqual([251, 191, 36, 255]);
    expect(marks.fillColors[3]).toBe(0);
    expect(Array.from(marks.fillColors.subarray(4, 8))).toEqual([74, 222, 128, 255]);
    expect(marks.lineColors[7]).toBe(0);
  });

  it('draws a never-read file with the thinner 0.6 px outline', () => {
    const files = encodeFiles(scene, visibility, style);
    expect(
      [files.lineWidths[2], files.lineWidths[4]].map((w) => Math.round((w ?? 0) * 10)),
    ).toEqual([6, 11]);
  });
});
