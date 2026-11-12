// Tests for which labels a frame offers and keeps on the seven-file tree, with text measured as
// 6 pixels a character so every box is known.
import { describe, expect, it } from 'vitest';
import { levelOfDetail } from '../layout/lod';
import { DARK_COLORS } from '../theme/tokens';
import { lookAt, NO_INSETS, toScreen } from './camera';
import type { FrameStyle } from './frameStyle';
import { neighbourhoodOf } from './highlight';
import { focusHubs } from './hubEncoding';
import { chooseLabels, spotOf, type LabelInputs } from './labelCandidates';
import { sevenFileScene, testLink } from './testScene';
import { computeVisibility } from './visibility';

const scene = sevenFileScene([testLink(0, 4, 'co_change', 0.9)]);
const SCREEN = { width: 1600, height: 1200 };

function inputs(zoom: number, focus: number | null): LabelInputs {
  const camera = lookAt(0, 0, zoom, NO_INSETS);
  const lod = levelOfDetail(scene.layout, zoom);
  const visibility = computeVisibility(scene.layout, lod, true);
  const neighbourhood = neighbourhoodOf(scene, focus, 0.25);
  const style: FrameStyle = {
    colors: DARK_COLORS,
    nowMs: 0,
    neighbourhood,
    selected: null,
    minStrength: 0.25,
  };
  return {
    scene,
    visibility,
    camera,
    screen: SCREEN,
    area: { left: 0, top: 0, ...SCREEN },
    chrome: [],
    neighbourhood,
    markedHubs: focusHubs(scene, visibility, style),
    measure: (text) => text.length * 6,
  };
}

describe('spotOf', () => {
  it('puts a 30 x 10 label 5 pixels below, above, right or left of a node at (100, 50)', () => {
    const size = { width: 30, height: 10, distance: 5 };
    expect(spotOf('below', 100, 50, size)).toEqual({ left: 85, top: 55, width: 30, height: 10 });
    expect(spotOf('above', 100, 50, size)).toEqual({ left: 85, top: 35, width: 30, height: 10 });
    expect(spotOf('right', 100, 50, size)).toEqual({ left: 105, top: 45, width: 30, height: 10 });
    expect(spotOf('left', 100, 50, size)).toEqual({ left: 65, top: 45, width: 30, height: 10 });
  });
});

describe('chooseLabels', () => {
  it('names every folder but the root, and no file, at zoom 0.5 (1.41 pixels a unit, below 2)', () => {
    // a, c and b/deep collapse (15 * 1.41 = 21 pixels, below 24) and are named with their file counts.
    const keys = chooseLabels(inputs(0.5, null)).map((label) => label.text);
    expect([...keys].sort()).toEqual(['a · 2', 'b', 'c · 1', 'deep · 1']);
  });

  it('names the files too at zoom 1 (2 pixels a unit)', () => {
    const texts = chooseLabels(inputs(1, null)).map((label) => label.text);
    expect(texts).toContain('README.md');
    expect(texts).toContain('only.rs');
  });

  it('names the focus file and its neighbour at zoom 0.5, first, and dims the others', () => {
    const labels = chooseLabels(inputs(0.5, 0));
    expect(labels.slice(0, 2).map((label) => label.text)).toEqual(['README.md', 'mid.rs']);
    expect(labels.slice(0, 2).every((label) => !label.isDimmed)).toBe(true);
    // Lit: README.md (f0), mid.rs (f4) and their hubs, the root (h0, unnamed) and b (h2).
    const lit = new Set(['f0', 'f4', 'h2']);
    expect(labels.map((label) => label.text)).toContain('a · 2');
    for (const label of labels) expect(label.isDimmed, label.text).toBe(!lit.has(label.key));
  });

  it('keeps placed boxes 4 pixels apart at least (2 of padding around each)', () => {
    // Text 14 pixels a character at zoom 1: the names are wider than the gaps between the nodes,
    // so without the rule several would overlap.
    const frame = { ...inputs(1, 0), measure: (text: string) => text.length * 14 };
    const labels = chooseLabels(frame);
    const boxes = labels.map((label) => {
      const [x, y] = toScreen(frame.camera, SCREEN, label.worldX, label.worldY);
      const distance = Math.max(Math.abs(label.offset[0]), Math.abs(label.offset[1]));
      const size = { width: label.text.length * 14, height: label.sizePx * 1.25, distance };
      return spotOf(label.side, x, y, size);
    });
    expect(boxes.length).toBeGreaterThan(4);
    for (const [i, a] of boxes.entries()) {
      for (const b of boxes.slice(i + 1)) {
        const apart =
          a.left + a.width + 4 <= b.left ||
          b.left + b.width + 4 <= a.left ||
          a.top + a.height + 4 <= b.top ||
          b.top + b.height + 4 <= a.top;
        expect(apart).toBe(true);
      }
    }
  });
});
