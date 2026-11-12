// The clipping rule with a measure of one pixel per character, so each expected text is counted by
// hand in the comment beside it.
import { describe, expect, it } from 'vitest';
import { clipText } from './clip';

// One pixel per UTF-16 unit: ASCII and the ellipsis are one unit each, an emoji two.
const perCharacter = (text: string): number => text.length;

describe('clipText', () => {
  it('keeps text that fits whole', () => {
    expect(clipText('src/orders/book.rs', 18, 'path', perCharacter)).toBe('src/orders/book.rs');
    expect(clipText('Keep the book', 13, 'words', perCharacter)).toBe('Keep the book');
  });

  it('cuts a path before its last segments, as many as fit', () => {
    // "…/orders/book.rs" is 1 + 1 + 6 + 1 + 7 = 16 characters; "…/book.rs" is 9.
    expect(clipText('src/orders/book.rs', 16, 'path', perCharacter)).toBe('…/orders/book.rs');
    expect(clipText('src/orders/book.rs', 15, 'path', perCharacter)).toBe('…/book.rs');
  });

  it('cuts a sentence after its first words, dropping a trailing comma or colon', () => {
    // "Keep cancelled orders…" is 22 characters, "Keep cancelled…" 15.
    expect(clipText('Keep cancelled orders in the book', 22, 'words', perCharacter)).toBe(
      'Keep cancelled orders…',
    );
    expect(clipText('Keep cancelled orders in the book', 21, 'words', perCharacter)).toBe(
      'Keep cancelled…',
    );
    // "Strongest coupling…" is 19 characters: the colon goes with the cut.
    expect(clipText('Strongest coupling: book.rs and mod.rs', 21, 'words', perCharacter)).toBe(
      'Strongest coupling…',
    );
  });

  it('cuts inside a word only when one word alone is too wide', () => {
    // "superca…" is 8 characters.
    expect(clipText('supercalifragilistic', 8, 'words', perCharacter)).toBe('superca…');
    // A path whose name alone is too wide keeps the start of the name: "veryl…" is 6.
    expect(clipText('a/verylongfilename.rs', 6, 'path', perCharacter)).toBe('veryl…');
  });

  it('never cuts a character outside the basic plane in half', () => {
    // "ab😀cd" is 6 UTF-16 units (the emoji is a pair). At width 4, "ab" plus half the pair plus
    // the ellipsis would be 4 units; whole characters give "ab😀…" (5, too wide), then "ab…" (3).
    expect(clipText('ab😀cd', 4, 'words', perCharacter)).toBe('ab…');
  });

  it('never returns text wider than the width while a cut exists', () => {
    for (let width = 2; width <= 40; width += 1) {
      const shown = clipText('src/billing/invoice/lines/total.rs', width, 'path', perCharacter);
      expect(perCharacter(shown)).toBeLessThanOrEqual(width);
    }
  });
});
