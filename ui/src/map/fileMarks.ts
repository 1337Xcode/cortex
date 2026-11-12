// The second channel of a file's mark, so no state is told by colour alone: a file with a suspect
// fact gets a second, outer ring; a file read or edited in the last 24 hours gets a filled dot in
// its centre; a never-read file is drawn with a thinner outline (set by the file encoder). Drawn
// as one layer of rings and dots over the files.
import { CANVAS_SIZE } from '../theme/tokens';
import { rgba } from '../theme/rgba';
import { fileMark } from './fileEncoding';
import { focusShare, writeRgba, type FrameStyle } from './frameStyle';
import type { HubBuffers } from './hubEncoding';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** The outer edge, in world units, of a file of radius `radius` with its suspect ring. */
export function suspectRingRadius(radius: number): number {
  return radius + CANVAS_SIZE.suspectRingGap;
}

/** The radius, in world units, of the recent dot in a file of radius `radius`. */
export function recentDotRadius(radius: number): number {
  return radius * CANVAS_SIZE.recentDotShare;
}

/** Rings and dots as binary attributes; row i marks file `ids[i]` (radius in world units). */
export function encodeFileMarks(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
): HubBuffers {
  const marked = Array.from(visibility.visibleFiles).filter((id) => {
    const file = scene.snapshot.files[id];
    if (file === undefined) return false;
    const mark = fileMark(file, style.nowMs, scene.hasReads);
    return mark === 'suspect' || mark === 'recent';
  });
  const count = marked.length;
  const buffers: HubBuffers = {
    ids: Uint32Array.from(marked),
    positions: new Float32Array(count * 2),
    radii: new Float32Array(count),
    fillColors: new Uint8ClampedArray(count * 4),
    lineColors: new Uint8ClampedArray(count * 4),
  };
  const suspect = rgba(style.colors.canvas.suspect);
  const recent = rgba(style.colors.canvas.recent);
  for (const [row, id] of marked.entries()) {
    const file = scene.snapshot.files[id];
    const radius = scene.fileRadius[id] ?? 0;
    const isSuspect = file !== undefined && file.suspect > 0;
    const share = focusShare(style.neighbourhood?.isMember[id] === 1, style);
    buffers.positions.set([scene.layout.fileX[id] ?? 0, scene.layout.fileY[id] ?? 0], row * 2);
    buffers.radii[row] = isSuspect ? suspectRingRadius(radius) : recentDotRadius(radius);
    // A ring has no fill; a dot has no outline.
    writeRgba(buffers.lineColors, row, suspect, isSuspect ? share : 0);
    writeRgba(buffers.fillColors, row, recent, isSuspect ? 0 : share);
  }
  return buffers;
}
