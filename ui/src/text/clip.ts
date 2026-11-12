// Shortens text to a width with an ellipsis, cutting at a boundary a reader can follow: a path
// keeps its last segments ("…/orders/book.rs", read from the end, where the name is), and a sentence
// or a name keeps its first words ("Keep cancelled orders…"). Only a single word too long for the
// width on its own is cut inside the word, as nothing else would fit. Pure: the width of a text
// comes from the measuring function given, so the rule is tested without a browser.

/** Measures a text's width in pixels. */
export type Measure = (text: string) => number;

/** How a text is read, which decides where it may be cut. */
export type ClipKind = 'path' | 'words';

/** The mark that stands for what was cut. */
export const ELLIPSIS = '…';

/**
 * The longest start of `text` that, with the ellipsis after it, fits `width`; never empty. It
 * counts code points, not UTF-16 units, so a character outside the basic plane (an emoji) is never
 * split into half a pair.
 */
function clipCharacters(text: string, width: number, measure: Measure): string {
  const characters = Array.from(text);
  const startOf = (length: number): string => characters.slice(0, length).join('');
  let length = characters.length - 1;
  while (length > 1 && measure(`${startOf(length)}${ELLIPSIS}`) > width) length -= 1;
  return `${startOf(Math.max(length, 1))}${ELLIPSIS}`;
}

/** The text cut after its first words, as many as fit with the ellipsis. */
function clipWords(text: string, width: number, measure: Measure): string {
  const words = text.split(' ');
  for (let count = words.length - 1; count >= 1; count -= 1) {
    const shown = `${words
      .slice(0, count)
      .join(' ')
      .replace(/[,;:.]$/, '')}${ELLIPSIS}`;
    if (measure(shown) <= width) return shown;
  }
  return clipCharacters(text, width, measure);
}

/** The path cut before its last segments, as many as fit after the ellipsis. */
function clipPath(path: string, width: number, measure: Measure): string {
  const segments = path.split('/');
  for (let count = segments.length - 1; count >= 1; count -= 1) {
    const shown = `${ELLIPSIS}/${segments.slice(-count).join('/')}`;
    if (measure(shown) <= width) return shown;
  }
  return clipWords(segments.at(-1) ?? path, width, measure);
}

/** `text` as it fits `width` pixels: whole when it fits, otherwise cut by the rule of its kind. */
export function clipText(text: string, width: number, kind: ClipKind, measure: Measure): string {
  if (measure(text) <= width) return text;
  return kind === 'path' ? clipPath(text, width, measure) : clipWords(text, width, measure);
}
