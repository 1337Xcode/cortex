// Measures label widths with the browser's own text metrics, once per text, size and weight, so
// the collision boxes match what the text layer draws.
import { FONT_FAMILY } from '../theme/tokens';
import type { MeasureText } from './labelCandidates';

/** Creates a measuring function backed by an off-screen 2D canvas and a cache. */
export function createTextMeasurer(): MeasureText {
  const context = document.createElement('canvas').getContext('2d');
  const cache = new Map<string, number>();
  return (text, sizePx, weight) => {
    const key = `${sizePx}|${weight}|${text}`;
    const known = cache.get(key);
    if (known !== undefined) return known;
    if (context === null) throw new Error('the browser gave no 2D canvas to measure labels with');
    context.font = `${weight} ${sizePx}px ${FONT_FAMILY}`;
    const width = Math.ceil(context.measureText(text).width);
    cache.set(key, width);
    return width;
  };
}
