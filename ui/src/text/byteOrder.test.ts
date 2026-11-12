// Tests for the UTF-8 byte-order comparison, checked against the encoder's actual bytes.
import { describe, expect, it } from 'vitest';
import { compareBytes, isStrictlySortedBytes } from './byteOrder';

/** Reference oracle: compare the UTF-8 encodings byte by byte. */
function referenceCompare(a: string, b: string): number {
  const bytesA = new TextEncoder().encode(a);
  const bytesB = new TextEncoder().encode(b);
  const shared = Math.min(bytesA.length, bytesB.length);
  for (let i = 0; i < shared; i += 1) {
    const diff = (bytesA[i] ?? 0) - (bytesB[i] ?? 0);
    if (diff !== 0) return diff;
  }
  return bytesA.length - bytesB.length;
}

const E_ACUTE = String.fromCodePoint(0xe9);
const HIGH_BMP = String.fromCodePoint(0xffee);
const ASTRAL_EMOJI = String.fromCodePoint(0x1f600);
const ASTRAL_FIRST = String.fromCodePoint(0x10000);

describe('compareBytes', () => {
  const samples = [
    '',
    'a',
    'a/b',
    'a-b/c',
    'A',
    'src/app.rs',
    `src/${E_ACUTE}.rs`,
    `src/${HIGH_BMP}.rs`,
    `src/${ASTRAL_EMOJI}.rs`,
    `src/${ASTRAL_FIRST}.rs`,
  ];

  it('agrees in sign with the UTF-8 bytes for every pair, astral characters included', () => {
    for (const a of samples) {
      for (const b of samples) {
        expect(Math.sign(compareBytes(a, b)), `${a} vs ${b}`).toBe(
          Math.sign(referenceCompare(a, b)),
        );
      }
    }
  });

  it('sorts a slash after a hyphen, as bytes do', () => {
    expect(compareBytes('a-b/c', 'a/b')).toBeLessThan(0);
  });

  it('puts a character above U+FFFF after U+FFEE although UTF-16 units say otherwise', () => {
    expect(ASTRAL_EMOJI < HIGH_BMP).toBe(true);
    expect(compareBytes(ASTRAL_EMOJI, HIGH_BMP)).toBeGreaterThan(0);
  });
});

describe('isStrictlySortedBytes', () => {
  it('accepts a sorted list and rejects an unsorted one or a duplicate', () => {
    expect(isStrictlySortedBytes(['a', 'b', 'c'])).toBe(true);
    expect(isStrictlySortedBytes(['b', 'a'])).toBe(false);
    expect(isStrictlySortedBytes(['a', 'a'])).toBe(false);
    expect(isStrictlySortedBytes([])).toBe(true);
  });
});
