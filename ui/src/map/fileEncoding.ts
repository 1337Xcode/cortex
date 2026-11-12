// Encodes the drawn files into typed arrays for one deck.gl layer: positions and radii from the
// layout, and an outline in the colour of the file's tone in the active view (in the default view,
// its mark: a suspect fact amber, read or edited in the last 24 hours green, never read while
// others were dim, or nothing neutral). A tone the view puts in the background draws thinner; in a
// view that tints, every other tone also fills the file with a share of its colour. The selected
// file is outlined in the accent; files outside the focus are dimmed.
import type { GraphFile } from '../api/graph';
import { CANVAS_SIZE, OPACITY, RECENT_WINDOW_MS } from '../theme/tokens';
import { rgbaOf } from '../theme/oklch';
import { toneColor } from '../views/view';
import { focusShare, mixRgba, writeRgba, type FrameStyle } from './frameStyle';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** What a file's outline says, most important first. */
export type FileMark = 'suspect' | 'recent' | 'unread' | 'plain';

/** The one mark a file's outline shows. Never-read only counts when some file has been read. */
export function fileMark(file: GraphFile, nowMs: number, hasReads: boolean): FileMark {
  if (file.suspect > 0) return 'suspect';
  if (file.touched_ms !== null && nowMs - file.touched_ms <= RECENT_WINDOW_MS) return 'recent';
  if (hasReads && file.reads === 0) return 'unread';
  return 'plain';
}

/** The drawn files as binary attributes; row i is file `ids[i]`. */
export interface FileBuffers {
  readonly ids: Uint32Array;
  readonly positions: Float32Array;
  readonly radii: Float32Array;
  readonly fillColors: Uint8ClampedArray;
  readonly lineColors: Uint8ClampedArray;
  readonly lineWidths: Float32Array;
}

/** Encodes the files `visibility` draws. */
export function encodeFiles(scene: Scene, visibility: Visibility, style: FrameStyle): FileBuffers {
  const ids = visibility.visibleFiles;
  const count = ids.length;
  const buffers: FileBuffers = {
    ids,
    positions: new Float32Array(count * 2),
    radii: new Float32Array(count),
    fillColors: new Uint8ClampedArray(count * 4),
    lineColors: new Uint8ClampedArray(count * 4),
    lineWidths: new Float32Array(count),
  };
  const fill = rgbaOf(style.colors.background);
  const accent = rgbaOf(style.colors.accent);
  const { toneOf, tones, isTinted } = style.view.encoding;
  const palette = tones.map((tone) => rgbaOf(toneColor(tone.role, style.colors)));
  const fills = tones.map((tone, index) => {
    const color = palette[index];
    return !isTinted || tone.isDim || color === undefined
      ? fill
      : mixRgba(fill, color, OPACITY.toneFill);
  });
  for (let row = 0; row < count; row += 1) {
    const id = ids[row] ?? 0;
    buffers.positions[row * 2] = scene.layout.fileX[id] ?? 0;
    buffers.positions[row * 2 + 1] = scene.layout.fileY[id] ?? 0;
    buffers.radii[row] = scene.fileRadius[id] ?? 0;
    const share = focusShare(style.neighbourhood?.isMember[id] === 1, style);
    const isSelected = id === style.selected;
    const tone = toneOf[id] ?? 0;
    // A file dimmed by the focus loses its tint with its outline's strength.
    writeRgba(buffers.fillColors, row, share < 1 ? fill : (fills[tone] ?? fill), 1);
    const color = palette[tone] ?? fill;
    writeRgba(buffers.lineColors, row, isSelected ? accent : color, share);
    const isFocus = isSelected || id === style.neighbourhood?.focus;
    const isDim = tones[tone]?.isDim === true;
    const restingPx = isDim ? CANVAS_SIZE.fileStrokeUnreadPx : CANVAS_SIZE.fileStrokePx;
    buffers.lineWidths[row] = isFocus ? CANVAS_SIZE.fileStrokeFocusedPx : restingPx;
  }
  return buffers;
}
