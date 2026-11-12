// Encodes the fact badges: a small dot on the upper right of the rim of every drawn file that has
// facts, neutral when every fact holds and amber when one is suspect (whose file also keeps its
// second ring, the shape that says so). The count is told on hover. Badges are drawn only once
// files are near enough to read their names, so the overview stays uncrowded.
import { CANVAS_SIZE, LABEL } from '../theme/tokens';
import { rgbaOf } from '../theme/oklch';
import { focusShare, writeRgba, type FrameStyle } from './frameStyle';
import type { HubBuffers } from './hubEncoding';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** Whether badges are drawn at `scale` pixels per world unit: with the file names, not before. */
export function areBadgesShown(visibility: Visibility, scale: number): boolean {
  return visibility.areFilesShown && scale >= LABEL.fileMinPixelsPerUnit;
}

/** Where a badge sits on a file of `radius` at (x, y): on its rim, up and to the right. */
export function badgeSpot(x: number, y: number, radius: number): [number, number] {
  const offset = radius * Math.SQRT1_2;
  return [x + offset, y - offset];
}

/** The badges of the drawn files that have facts; row i is file `ids[i]`, radius in pixels. */
export function encodeBadges(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  isShown: boolean,
): HubBuffers {
  const { files } = scene.snapshot;
  const marked = isShown
    ? Array.from(visibility.visibleFiles).filter((id) => (files[id]?.facts ?? 0) > 0)
    : [];
  const buffers: HubBuffers = {
    ids: Uint32Array.from(marked),
    positions: new Float32Array(marked.length * 2),
    radii: new Float32Array(marked.length),
    fillColors: new Uint8ClampedArray(marked.length * 4),
    lineColors: new Uint8ClampedArray(marked.length * 4),
  };
  const plain = rgbaOf(style.colors.canvas.badge);
  const suspect = rgbaOf(style.colors.canvas.suspect);
  const edge = rgbaOf(style.colors.background);
  for (const [row, id] of marked.entries()) {
    const spot = badgeSpot(
      scene.layout.fileX[id] ?? 0,
      scene.layout.fileY[id] ?? 0,
      scene.fileRadius[id] ?? 0,
    );
    const share = focusShare(style.neighbourhood?.isMember[id] === 1, style);
    buffers.positions.set(spot, row * 2);
    buffers.radii[row] = CANVAS_SIZE.badgeRadiusPx;
    writeRgba(buffers.fillColors, row, (files[id]?.suspect ?? 0) > 0 ? suspect : plain, share);
    writeRgba(buffers.lineColors, row, edge, share);
  }
  return buffers;
}
