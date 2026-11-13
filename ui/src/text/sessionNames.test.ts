// Tests for the names sessions go by: the shortest start of an id no other session shares, at least
// four characters, with every expected name written out.
import { describe, expect, it } from 'vitest';
import { sessionNames } from './sessionNames';

/** Two Codex ids as a real daemon gave them: UUIDv7, so the first characters are a timestamp. */
const FIRST = '01a1d373-f6af-7e0b-8c2d-5a4b3c2d1e0f';
const SECOND = '01a1d375-b58f-7a41-9b3e-6c5d4e3f2a1b';

describe('sessionNames', () => {
  it('tells two ids that start alike apart at the first character they differ in', () => {
    const names = sessionNames([FIRST, SECOND]);
    expect([names.nameOf(FIRST), names.nameOf(SECOND)]).toEqual(['01a1d373…', '01a1d375…']);
  });

  it('keeps four characters for an id no other starts with', () => {
    const names = sessionNames(['3f2a9c1e-0000', FIRST, SECOND]);
    expect(names.nameOf('3f2a9c1e-0000')).toBe('3f2a…');
    expect(names.nameOf(FIRST)).toBe('01a1d373…');
  });

  it('names a session alone by four characters, and a short id whole', () => {
    expect(sessionNames(['3f2a9c1e']).nameOf('3f2a9c1e')).toBe('3f2a…');
    const names = sessionNames(['3f2a', 'ab', '3f2b9c1e']);
    expect([names.nameOf('3f2a'), names.nameOf('ab'), names.nameOf('3f2b9c1e')]).toEqual([
      '3f2a',
      'ab',
      '3f2b…',
    ]);
  });

  it('gives three ids that share six characters the seventh', () => {
    const ids = ['abcdef1-x', 'abcdef2-x', 'abcdef3-x'];
    const names = sessionNames(ids);
    expect(ids.map((id) => names.nameOf(id))).toEqual(['abcdef1…', 'abcdef2…', 'abcdef3…']);
  });

  it('lets only the ids that share a start lengthen each other, not a distant third', () => {
    // `abcd1` and `abcd2` share four characters; `zzzz` shares none with either.
    const names = sessionNames(['abcd1-long', 'abcd2-long', 'zzzz-long']);
    expect([
      names.nameOf('abcd1-long'),
      names.nameOf('abcd2-long'),
      names.nameOf('zzzz-long'),
    ]).toEqual(['abcd1…', 'abcd2…', 'zzzz…']);
  });

  it('names an id whole when it is the start of another, and the other one past it', () => {
    const names = sessionNames(['abcdef', 'abcdefgh']);
    expect([names.nameOf('abcdef'), names.nameOf('abcdefgh')]).toEqual(['abcdef', 'abcdefg…']);
  });

  it('names an id by one name however many times the page holds it', () => {
    const names = sessionNames([FIRST, SECOND, FIRST, SECOND, FIRST]);
    expect(names.nameOf(FIRST)).toBe('01a1d373…');
  });

  it('never cuts inside a character outside the Basic Plane', () => {
    const names = sessionNames(['\u{1F600}\u{1F601}\u{1F602}\u{1F603}\u{1F604}', 'x']);
    expect(names.nameOf('\u{1F600}\u{1F601}\u{1F602}\u{1F603}\u{1F604}')).toBe(
      '\u{1F600}\u{1F601}\u{1F602}\u{1F603}…',
    );
  });

  it('names an id the page does not know of whole', () => {
    expect(sessionNames([FIRST]).nameOf('3f2a9c1e-unknown')).toBe('3f2a9c1e-unknown');
  });
});
