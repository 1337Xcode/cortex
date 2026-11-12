// Tests for the shared guard pieces that render untrusted text.
import { describe, expect, it } from 'vitest';
import { isRepoPath, visibleText } from './guard';

describe('hidden formatting characters', () => {
  it('are shown as the replacement character in text', () => {
    expect(visibleText('fix \u202eexe.txt\u202c now')).toBe('fix \ufffdexe.txt\ufffd now');
    expect(visibleText('a\u200bb\u2066c\u2069\ufeff')).toBe('a\ufffdb\ufffdc\ufffd\ufffd');
    expect(visibleText('plain \u00e9 text')).toBe('plain \u00e9 text');
  });

  it('are refused in paths, at both ends of each range', () => {
    for (const code of [0x200b, 0x200f, 0x202a, 0x202e, 0x2066, 0x2069, 0xfeff]) {
      expect(isRepoPath(`src/a${String.fromCodePoint(code)}b.rs`), code.toString(16)).toBe(false);
    }
    expect(isRepoPath('src/a\u2010b.rs')).toBe(true);
  });
});
