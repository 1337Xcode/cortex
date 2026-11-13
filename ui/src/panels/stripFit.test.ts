// The strip's fitting rule with a measure of one pixel per character and a gap of one, so every
// expected line is worked out by hand in the comments.
import { describe, expect, it } from 'vitest';
import { fitStrip, fullKeys, type StripText } from './stripFit';
import type { SentencePart, SummarySentence } from './summarySentences';

const SENTENCES: SummarySentence[] = [
  { key: 'pair', parts: ['AAAAAAAAAA'], short: ['AAAA'] },
  { key: 'busiest', parts: ['BBBBBB'], short: ['BB'] },
  { key: 'indexed', parts: ['CCCCCCCCCCCC'], short: ['CCC'] },
];

const TEXT: StripText = {
  textOf: (parts: readonly SentencePart[]) =>
    parts.map((part) => (typeof part === 'string' ? part : part.path)).join(''),
  measure: (text: string) => text.length,
};

function shown(width: number): string[] | string {
  const line = fitStrip(SENTENCES, width, 1, TEXT);
  return line.kind === 'cut' ? line.text : line.sentences.map((s) => TEXT.textOf(s.parts));
}

describe('fitStrip', () => {
  it('shows every sentence in full when they fit', () => {
    // 10, then 6 + 1, then 12 + 1: 30 exactly.
    expect(shown(30)).toEqual(['AAAAAAAAAA', 'BBBBBB', 'CCCCCCCCCCCC']);
  });

  it('leaves out a later sentence that fits in neither form', () => {
    // 10 and 7 leave 3; C needs 13 in full and 4 short.
    expect(shown(20)).toEqual(['AAAAAAAAAA', 'BBBBBB']);
  });

  it('uses the short forms when the full ones do not fit', () => {
    // A full is 10 > 8, short 4 leaves 4; B full needs 7, short 3 leaves 1; C needs 4.
    expect(shown(8)).toEqual(['AAAA', 'BB']);
  });

  it("cuts the first sentence's short form when nothing fits whole", () => {
    expect(shown(3)).toBe('AAAA');
  });
});

describe('fullKeys', () => {
  it('names the sentences shown in full, not those shown short', () => {
    // At 19: "AAAAAAAAAA" (10) and "BBBBBB" (6 + 1, 17); "CCC" (3 + 1) would make 21.
    expect(fullKeys(fitStrip(SENTENCES, 19, 1, TEXT), SENTENCES)).toBe('pair busiest');
    // At 16: "AAAAAAAAAA" (10), "BB" short (2 + 1, 13) and "CCC" short (3 + 1, 17) does not fit.
    expect(fullKeys(fitStrip(SENTENCES, 16, 1, TEXT), SENTENCES)).toBe('pair');
  });

  it('names none when the line is cut', () => {
    // At 3 not even "AAAA", the first short form, fits.
    expect(fullKeys(fitStrip(SENTENCES, 3, 1, TEXT), SENTENCES)).toBe('');
  });
});
