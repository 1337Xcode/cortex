// Measures text with the browser's own font metrics, once per text and font, so a label's collision
// box on the canvas and a clipped line in a panel match what is drawn. One shared measurer, backed
// by an off-screen 2D canvas.

/** Measures the width in pixels of `text` in a CSS font (`600 12px "Geist", sans-serif`). */
export type MeasureFont = (text: string, font: string) => number;

let measurer: MeasureFont | null = null;

function createMeasurer(): MeasureFont {
  const context = document.createElement('canvas').getContext('2d');
  const cache = new Map<string, number>();
  return (text, font) => {
    const key = `${font}|${text}`;
    const known = cache.get(key);
    if (known !== undefined) return known;
    if (context === null) throw new Error('the browser gave no 2D canvas to measure text with');
    context.font = font;
    const width = context.measureText(text).width;
    cache.set(key, width);
    return width;
  };
}

/**
 * The page's measurer. Call it only once the fonts have settled: a width measured in a fallback
 * font would be cached and stay wrong.
 */
export function fontMeasurer(): MeasureFont {
  measurer ??= createMeasurer();
  return measurer;
}
