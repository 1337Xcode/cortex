// Shortens text to a width with an ellipsis, cutting at a boundary a reader can follow: a path
// keeps its last segments ("…/orders/book.rs", read from the end, where the name is), and a sentence
// or a name keeps its first words ("Keep cancelled orders…"). Only a single word too long for the
// width on its own is cut inside the word, as nothing else would fit. Two paths in one row ("a and
// b"), and a few words before a path, are cut so the end of each path stays in view. Pure: the width
// of a text comes from the measuring function given, so the rule is tested without a browser.

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
      .replace(/[,;:.…]$/, '')}${ELLIPSIS}`;
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

/** What stands between a lead and the path after it. */
export const LEAD_JOINER = ' ';

/**
 * A few words then a path ("Codex 9b1c… reading src/billing/invoice.rs") in `width` pixels: whole
 * when they fit; else the words keep all the room the path's shortest end ("…/invoice.rs", or the
 * whole path when it has no folder) leaves them, cut after their first words when even that is
 * too much, and the path is cut as a path to what the words left.
 */
export function clipLeadAndPath(
  lead: string,
  path: string,
  width: number,
  measure: Measure,
): readonly [string, string] {
  if (measure(`${lead}${LEAD_JOINER}${path}`) <= width) return [lead, path];
  const room = width - measure(LEAD_JOINER);
  const slash = path.lastIndexOf('/');
  const shortestEnd = slash < 0 ? path : `${ELLIPSIS}${path.slice(slash)}`;
  const leadShown = clipText(lead, room - measure(shortestEnd), 'words', measure);
  return [leadShown, clipText(path, room - measure(leadShown), 'path', measure)];
}

/** What stands between the two paths of a pair. */
export const PAIR_JOINER = ' and ';

/**
 * Two paths shown as "a and b" in `width` pixels, each cut as a path, so the end of both (where
 * their names are) stays in view: whole when the pair fits; else the first is cut to half the room
 * (whole when it fits there), the second to what the first left, and the first again to what the
 * second left, so room one path does not need goes to the other.
 */
export function clipPair(
  first: string,
  second: string,
  width: number,
  measure: Measure,
): readonly [string, string] {
  if (measure(`${first}${PAIR_JOINER}${second}`) <= width) return [first, second];
  const room = width - measure(PAIR_JOINER);
  const firstHalf = clipText(first, room / 2, 'path', measure);
  const secondShown = clipText(second, room - measure(firstHalf), 'path', measure);
  return [clipText(first, room - measure(secondShown), 'path', measure), secondShown];
}
