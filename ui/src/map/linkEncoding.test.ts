// Tests for the learned-link encoder on the seven-file tree: the threshold, ends drawn at the
// collapsed hub that hides a file, links inside one collapsed hub dropped, the focus drawn last.
import { describe, expect, it } from 'vitest';
import { levelOfDetail } from '../layout/lod';
import { DARK_COLORS } from '../theme/tokens';
import type { FrameStyle } from './frameStyle';
import { neighbourhoodOf } from './highlight';
import { countLinksByKind, dashOf, encodeLinks, linkOpacity } from './linkEncoding';
import { boxKey, nearBox, prominentLinks } from './linkProminence';
import { sevenFileScene, testLink } from './testScene';
import { computeVisibility } from './visibility';

// 0 README-a/one co-change 0.9; 1 a/one-a/two used together 0.1 (below the 0.25 threshold);
// 2 a/one-a/two work order 0.5; 3 b/mid-b/zed work order 0.5.
const LINKS = [
  testLink(0, 1, 'co_change', 0.9),
  testLink(1, 2, 'used_together', 0.1),
  testLink(1, 2, 'work_order', 0.5),
  testLink(4, 5, 'work_order', 0.5),
];
const scene = sevenFileScene(LINKS);
const style: FrameStyle = {
  colors: DARK_COLORS,
  nowMs: 0,
  neighbourhood: null,
  selected: null,
  minStrength: 0.25,
};

function visibilityAt(zoom: number) {
  return computeVisibility(scene.layout, levelOfDetail(scene.layout, zoom), true);
}

describe('link opacity and dash', () => {
  it('runs from 0.2 at weight 0 to 0.6 at weight 1', () => {
    expect(linkOpacity(0)).toBeCloseTo(0.2, 9);
    expect(linkOpacity(0.5)).toBeCloseTo(0.4, 9);
    expect(linkOpacity(1)).toBeCloseTo(0.6, 9);
  });

  it('draws co-change solid, used together dashed and work orders dotted', () => {
    expect([dashOf('co_change'), dashOf('used_together'), dashOf('work_order')]).toEqual([
      [0, 0],
      [6, 4],
      [1.5, 3.5],
    ]);
  });
});

describe('encodeLinks', () => {
  it('draws the links at or above the threshold between the files when all are shown', () => {
    const lines = encodeLinks(scene, visibilityAt(1), style, null);
    expect(Array.from(lines.ids)).toEqual([0, 2, 3]);
    // README (10, 0) to a/one.rs (35.696000525, 20).
    expect(lines.sources[0]).toBeCloseTo(10, 4);
    expect(lines.targets[1]).toBeCloseTo(20, 4);
    // Alpha 0.2 + 0.4 * 0.9 = 0.56 of 255 = 143; weight 0.5 gives 0.4 of 255 = 102.
    expect([lines.colors[3], lines.colors[7]]).toEqual([143, 102]);
    expect(Array.from(lines.dashes.subarray(0, 4))).toEqual([0, 0, 1.5, 3.5]);
  });

  it('ends a link at the collapsed hub and drops one inside a single collapsed hub', () => {
    // At zoom 0, a (hub 1, at 25.696000525, 20) is collapsed: link 2 joins two of its files.
    const lines = encodeLinks(scene, visibilityAt(0), style, null);
    expect(Array.from(lines.ids)).toEqual([0, 3]);
    expect(lines.targets[0]).toBeCloseTo(25.696, 3);
    expect(lines.targets[1]).toBeCloseTo(20, 4);
  });

  it('draws the focus file links last, at 0.95, and dims the rest to 0.18', () => {
    const neighbourhood = neighbourhoodOf(scene, 4, 0.25);
    const lines = encodeLinks(scene, visibilityAt(1), { ...style, neighbourhood }, null);
    expect(Array.from(lines.ids)).toEqual([0, 2, 3]);
    // Link 3 touches b/mid.rs: 0.95 of 255 = 242, width 2. Link 0: 0.56 * 0.18 of 255 = 25.7.
    expect([lines.colors[11], lines.widths[2]]).toEqual([242, 2]);
    expect(lines.colors[3]).toBe(26);
  });

  it('puts the focus links after the others when they come first in link order', () => {
    const neighbourhood = neighbourhoodOf(scene, 0, 0.25);
    const lines = encodeLinks(scene, visibilityAt(1), { ...style, neighbourhood }, null);
    expect(Array.from(lines.ids)).toEqual([2, 3, 0]);
  });
});

describe('countLinksByKind', () => {
  it('counts the links at or above the threshold, whatever the zoom', () => {
    expect(countLinksByKind(LINKS, 0.25)).toEqual({
      co_change: 1,
      used_together: 0,
      work_order: 2,
    });
    expect(countLinksByKind(LINKS, 0)).toEqual({ co_change: 1, used_together: 1, work_order: 2 });
  });
});

describe('link prominence', () => {
  // Every pair of the seven files as a co-change link, weight (7 * from + to) / 100, in link
  // order: ids 0-5 from file 0, 6-10 from 1, 11-14 from 2, 15-17 from 3, 18 (4,5), 19 (4,6),
  // 20 (5,6). The strongest are 20 (0.41), 19 (0.34) and 18 (0.33).
  const pairs: [number, number][] = [];
  for (let from = 0; from < 7; from += 1)
    for (let to = from + 1; to < 7; to += 1) pairs.push([from, to]);
  const dense = sevenFileScene(
    pairs.map(([from, to]) => testLink(from, to, 'co_change', (7 * from + to) / 100)),
  );
  const everywhere = { minX: -1e6, minY: -1e6, maxX: 1e6, maxY: 1e6 };
  const all = computeVisibility(dense.layout, levelOfDetail(dense.layout, 1), true);

  it('keeps every link strong while the frame is sparse', () => {
    const limits = { denseAbove: 21, strongestPerKind: 3 };
    expect(prominentLinks(dense, all, { minStrength: 0, box: everywhere, limits })).toBeNull();
  });

  it('keeps only the strongest few of a kind once dense', () => {
    const limits = { denseAbove: 20, strongestPerKind: 3 };
    const marked = prominentLinks(dense, all, { minStrength: 0, box: everywhere, limits });
    const strong = Array.from(marked ?? []).flatMap((flag, id) => (flag === 1 ? [id] : []));
    expect(strong).toEqual([18, 19, 20]);
  });

  it('counts only links with an end near the screen', () => {
    // A box around b's files only, (-60, 0) to (-55, 9): links touching files 4 and 5.
    const box = { minX: -61, minY: -1, maxX: -54, maxY: 9 };
    const limits = { denseAbove: 10, strongestPerKind: 3 };
    // Files 4 and 5 have 6 + 6 - 1 = 11 links, more than 10: (4,5) 0.33, (4,6) 0.34, (5,6) 0.41.
    const marked = prominentLinks(dense, all, { minStrength: 0, box, limits });
    expect(Array.from(marked ?? []).flatMap((flag, id) => (flag === 1 ? [id] : []))).toEqual([
      18, 19, 20,
    ]);
  });

  it('draws the rest faint (0.05 of 255 = 13), under the strong ones', () => {
    const marked = new Uint8Array(21);
    marked[20] = 1;
    const lines = encodeLinks(dense, all, { ...style, minStrength: 0 }, marked);
    expect(lines.ids[20]).toBe(20);
    expect([lines.colors[3], lines.colors[20 * 4 + 3]]).toEqual([
      13,
      Math.round(255 * (0.2 + 0.4 * 0.41)),
    ]);
  });

  it('moves the box only in quarter-screen steps', () => {
    const screen = { width: 800, height: 600 };
    // Zoom 0: a quarter screen is 200 world units across.
    expect(boxKey(nearBox({ x: 90, y: 0, zoom: 0 }, screen))).toBe(
      boxKey(nearBox({ x: 0, y: 0, zoom: 0 }, screen)),
    );
    expect(nearBox({ x: 110, y: 0, zoom: 0 }, screen)).toEqual({
      minX: -600,
      minY: -600,
      maxX: 1000,
      maxY: 600,
    });
  });
});
