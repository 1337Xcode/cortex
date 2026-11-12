// The inputs every encoder shares for one frame: the theme's colours, the moment "recent" is
// measured from, the focus neighbourhood and the selection. Plus the small writer the encoders use
// to put a colour into a typed array.
import type { ThemeColors } from '../theme/tokens';
import { OPACITY } from '../theme/tokens';
import type { Rgba } from '../theme/oklch';
import type { Neighbourhood } from './highlight';

/** The shared inputs of one frame's encoders. */
export interface FrameStyle {
  readonly colors: ThemeColors;
  /** Milliseconds since the epoch that "in the last 24 hours" is measured back from. */
  readonly nowMs: number;
  /** The hovered or selected file and its neighbours, or null when nothing is in focus. */
  readonly neighbourhood: Neighbourhood | null;
  readonly selected: number | null;
  /** Learned links below this weight are not drawn. */
  readonly minStrength: number;
}

/** The alpha share for something inside (1) or outside (`OPACITY.dimmed`) the focus. */
export function focusShare(isInFocus: boolean, style: FrameStyle): number {
  return style.neighbourhood === null || isInFocus ? 1 : OPACITY.dimmed;
}

/** Writes `color` into `target` at row `index` (4 channels), scaling its alpha by `alphaShare`. */
export function writeRgba(
  target: Uint8ClampedArray,
  index: number,
  color: Rgba,
  alphaShare: number,
): void {
  const at = index * 4;
  target[at] = color[0];
  target[at + 1] = color[1];
  target[at + 2] = color[2];
  target[at + 3] = Math.round(color[3] * alphaShare);
}
