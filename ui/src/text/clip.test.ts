// The clipping rule with a measure of one pixel per character, so each expected text is counted by
// hand in the comment beside it.
import { describe, expect, it } from 'vitest';
import { clipLeadAndPath, clipPair, clipText } from './clip';

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
    // A last kept word that ends in an ellipsis keeps only that one: "Claude Code s000…" is 17.
    expect(clipText('Claude Code s000… editing', 17, 'words', perCharacter)).toBe(
      'Claude Code s000…',
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

describe('clipLeadAndPath', () => {
  it('keeps words and a path that fit whole', () => {
    // "Codex 9b1c… reading invoice.rs" is 19 + 1 + 10 = 30 characters.
    expect(clipLeadAndPath('Codex 9b1c… reading', 'invoice.rs', 30, perCharacter)).toEqual([
      'Codex 9b1c… reading',
      'invoice.rs',
    ]);
  });

  it("keeps the words whole and cuts the path to what they leave, the path's name in view", () => {
    // Width 40 leaves 39 beside the space; "…/file149.rs" (12) leaves the 25 words their room, and
    // the path gets 14: "…/mod3/file149.rs" (17) does not fit, "…/file149.rs" (12) does.
    expect(
      clipLeadAndPath('Claude Code s000… editing', 'pkg0/mod3/file149.rs', 40, perCharacter),
    ).toEqual(['Claude Code s000… editing', '…/file149.rs']);
  });

  it('cuts the words when even the shortest end of the path leaves them too little', () => {
    // Width 24 leaves 23; "…/file149.rs" (12) leaves 11 for the words: "Claude…" (7). The path
    // then gets 16: "…/file149.rs" (12), "…/mod3/file149.rs" (17) being too long.
    expect(
      clipLeadAndPath('Claude Code s000… editing', 'pkg0/mod3/file149.rs', 24, perCharacter),
    ).toEqual(['Claude…', '…/file149.rs']);
  });
});

describe('clipPair', () => {
  it('keeps a pair that fits whole', () => {
    // "book.rs and mod.rs" is 7 + 5 + 6 = 18 characters.
    expect(clipPair('book.rs', 'mod.rs', 18, perCharacter)).toEqual(['book.rs', 'mod.rs']);
  });

  it('keeps a path that fits half the room whole and cuts the other to the rest', () => {
    // Width 20 leaves 15 beside " and ", half of it 7.5: "book.rs" (7) stays, and the other path
    // gets 8, where "…/mod.rs" (8) fits and "…/d/mod.rs" (10) does not.
    expect(clipPair('book.rs', 'a/b/c/d/mod.rs', 20, perCharacter)).toEqual([
      'book.rs',
      '…/mod.rs',
    ]);
    expect(clipPair('a/b/c/d/mod.rs', 'book.rs', 20, perCharacter)).toEqual([
      '…/mod.rs',
      'book.rs',
    ]);
  });

  it('gives the first path the room the second does not need', () => {
    // Width 20 leaves 15: cut to half (7.5) the first is "longna…" (7), the second "c/d.rs" (6)
    // fits whole, and the first takes the 9 left: "longname…".
    expect(clipPair('aa/bb/longname.rs', 'c/d.rs', 20, perCharacter)).toEqual([
      'longname…',
      'c/d.rs',
    ]);
  });

  it('cuts both paths to half the room when neither fits it, keeping both names', () => {
    // Width 40 leaves 35, half of it 17.5: "…/mod2/file21.rs" (16) and "…/mod4/file142.rs" (17).
    const pair = clipPair('pkg0/mod0/mod2/file21.rs', 'pkg1/mod4/file142.rs', 40, perCharacter);
    expect(pair).toEqual(['…/mod2/file21.rs', '…/mod4/file142.rs']);
    expect(perCharacter(`${pair[0]} and ${pair[1]}`)).toBeLessThanOrEqual(40);
  });
});
