// Tests for the camera moves and the dot grid, with values worked out in the comments.
import { describe, expect, it } from 'vitest';
import { buildScene } from './scene';
import {
  cameraOnFile,
  fitCamera,
  formatZoom,
  lookAt,
  NO_INSETS,
  stepZoom,
  toScreen,
  zoomPercent,
  ZOOM_STEP,
} from './camera';
import { gridDots } from './grid';
import { sevenFileScene, sevenFileSnapshot } from './testScene';

const SCREEN = { width: 800, height: 600 };

describe('camera', () => {
  it('steps the scale by 1.3 and shows 100% at zoom 0', () => {
    expect(zoomPercent(0)).toBe(100);
    expect(zoomPercent(stepZoom({ x: 0, y: 0, zoom: 0 }, 1).zoom)).toBe(130);
    expect(zoomPercent(stepZoom({ x: 0, y: 0, zoom: ZOOM_STEP }, -1).zoom)).toBe(100);
  });

  it('shows one decimal below 10% and whole percents above', () => {
    expect([
      formatZoom(Math.log2(0.014)),
      formatZoom(Math.log2(0.0996)),
      formatZoom(Math.log2(2.864)),
    ]).toEqual(['1.4%', '10.0%', '286%']);
  });

  it('never shows a zoom as 0.0%', () => {
    // The furthest zoom, 2^-14 = 0.0061%, would round to 0.0%; 0.05% is the least that shows 0.1%.
    expect([formatZoom(-14), formatZoom(Math.log2(0.0005))]).toEqual(['<0.1%', '0.1%']);
  });

  it('stops at the zoom limits', () => {
    expect(stepZoom({ x: 0, y: 0, zoom: 5 }, 1).zoom).toBe(5);
    expect(stepZoom({ x: 0, y: 0, zoom: -14 }, -1).zoom).toBe(-14);
  });

  it('puts a point at the centre of what the chrome leaves visible', () => {
    // A 100-pixel top bar: the visible centre is (400, 100 + 500 / 2) = (400, 350).
    const insets = { ...NO_INSETS, top: 100 };
    const camera = lookAt(10, 0, 1, insets);
    expect(toScreen(camera, SCREEN, 10, 0)).toEqual([400, 350]);
  });

  it('fits the whole graph inside the padded visible area, touching it on one axis', () => {
    const scene = sevenFileScene();
    const insets = { top: 80, right: 200, bottom: 0, left: 0 };
    const camera = fitCamera(scene, SCREEN, insets);
    const { minX, minY, maxX, maxY } = scene.bounds;
    const [left, top] = toScreen(camera, SCREEN, minX, minY);
    const [right, bottom] = toScreen(camera, SCREEN, maxX, maxY);
    // Visible: x 0..600, y 80..600; padding 52 on every side.
    expect(left).toBeGreaterThanOrEqual(52 - 1e-6);
    expect(right).toBeLessThanOrEqual(548 + 1e-6);
    expect(top).toBeGreaterThanOrEqual(132 - 1e-6);
    expect(bottom).toBeLessThanOrEqual(548 + 1e-6);
    const touchesX = Math.abs(left - 52) < 1e-3 && Math.abs(right - 548) < 1e-3;
    const touchesY = Math.abs(top - 132) < 1e-3 && Math.abs(bottom - 548) < 1e-3;
    expect(touchesX || touchesY).toBe(true);
  });

  it('never fits a small repository closer than 800%', () => {
    const one = buildScene({
      ...sevenFileSnapshot(),
      files: sevenFileSnapshot().files.slice(0, 1),
    });
    expect(zoomPercent(fitCamera(one, SCREEN, NO_INSETS).zoom)).toBe(800);
  });

  it('flies to a file at 400% at least, or closer when already closer', () => {
    const scene = sevenFileScene();
    // a/one.rs at (35.696000525, 20); its hub opens at log2(24 / 15) = 0.68, below 2.
    const flown = cameraOnFile(scene, { x: 0, y: 0, zoom: -1 }, 1, NO_INSETS);
    expect(flown.zoom).toBe(2);
    expect(flown.x).toBeCloseTo(35.696, 3);
    expect(cameraOnFile(scene, { x: 0, y: 0, zoom: 3 }, 1, NO_INSETS).zoom).toBe(3);
  });
});

describe('gridDots', () => {
  it('lays one dot every 16 pixels over the screen, plus a margin column and row', () => {
    // 32 x 16 pixels at zoom 0: 32 / 16 + 2 = 4 columns, 16 / 16 + 2 = 3 rows, from (-16, -16).
    const dots = gridDots({ x: 0, y: 0, zoom: 0 }, { width: 32, height: 16 });
    expect(dots.length).toBe(4 * 3 * 2);
    expect(Array.from(dots.subarray(0, 4))).toEqual([-16, -16, 0, -16]);
  });

  it('keeps 16 pixels between dots at any zoom (8 world units at zoom 1)', () => {
    const dots = gridDots({ x: 0, y: 0, zoom: 1 }, { width: 32, height: 16 });
    expect((dots[2] ?? 0) - (dots[0] ?? 0)).toBe(8);
  });
});
