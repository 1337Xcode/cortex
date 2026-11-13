// Tests for a file an answer named, found in the snapshot on screen: by its id only where the
// snapshot holds the same path there, else by its path, else shown by its path; and the words a row
// adds for a file it cannot link to.
import { describe, expect, it } from 'vitest';
import { fileIdOf } from '../api/graph';
import { namedFileNote, namedFileSentence, shownName } from '../text/namedFileWords';
import { namesOfFiles } from '../text/pathNames';
import { isOutOfStep, listedId, nameFile, namedPath } from './namedFile';

// The snapshot on screen, sorted by path as the guard keeps it.
const FILES = [{ path: 'a.rs' }, { path: 'src/book.rs' }, { path: 'src/mod.rs' }, { path: 'z.rs' }];

describe('finding a file by its path', () => {
  it('finds every path at its id and none that is not there', () => {
    expect(FILES.map((file) => fileIdOf(FILES, file.path))).toEqual([0, 1, 2, 3]);
    expect(['', 'b.rs', 'src/new.rs', 'zz.rs'].map((path) => fileIdOf(FILES, path))).toEqual([
      null,
      null,
      null,
      null,
    ]);
    expect(fileIdOf([], 'a.rs')).toBeNull();
  });
});

describe('a file an answer named', () => {
  it('is linked at its id when the snapshot holds its path there', () => {
    expect(nameFile(FILES, 1, 'src/book.rs')).toEqual({
      kind: 'listed',
      id: 1,
      path: 'src/book.rs',
    });
    expect(isOutOfStep(FILES, 1, 'src/book.rs')).toBe(false);
  });

  it('is found again by its path when the ids moved, and says the snapshot is out of step', () => {
    // The daemon added src/a.rs, so book.rs moved from id 1 to id 2 in its answer.
    expect(nameFile(FILES, 2, 'src/book.rs')).toEqual({
      kind: 'listed',
      id: 1,
      path: 'src/book.rs',
    });
    expect(isOutOfStep(FILES, 2, 'src/book.rs')).toBe(true);
  });

  it('is never linked to another file at its id when the snapshot does not hold it yet', () => {
    // Id 1 holds src/book.rs on screen; the daemon's id 1 is a new file.
    const named = nameFile(FILES, 1, 'src/aaa.rs');
    expect(named).toEqual({ kind: 'pending', path: 'src/aaa.rs' });
    expect([listedId(named), namedPath(named)]).toEqual([null, 'src/aaa.rs']);
    expect(isOutOfStep(FILES, 1, 'src/aaa.rs')).toBe(true);
    // An id past the snapshot's end, likewise.
    expect(nameFile(FILES, 9, 'src/new.rs')).toEqual({ kind: 'pending', path: 'src/new.rs' });
    expect(isOutOfStep(FILES, 9, 'src/new.rs')).toBe(true);
  });

  it('is shown by its path when the map does not list it, and asks for nothing', () => {
    expect(nameFile(FILES, null, 'vendor/gen.rs')).toEqual({
      kind: 'unlisted',
      path: 'vendor/gen.rs',
    });
    expect(isOutOfStep(FILES, null, 'vendor/gen.rs')).toBe(false);
  });

  it('is unknown without a path, whatever its id, and asks for nothing', () => {
    expect([nameFile(FILES, 1, null), nameFile(FILES, null, null)]).toEqual([
      { kind: 'unknown' },
      { kind: 'unknown' },
    ]);
    expect(isOutOfStep(FILES, 1, null)).toBe(false);
  });
});

describe('the words for a named file', () => {
  const names = namesOfFiles(FILES);

  it('name a listed file by the name the map gives it, any other by its whole path, an unknown not at all', () => {
    expect(
      [
        nameFile(FILES, 1, 'src/book.rs'),
        nameFile(FILES, 1, 'src/aaa.rs'),
        nameFile(FILES, null, 'vendor/gen.rs'),
        nameFile(FILES, null, null),
      ].map((named) => [shownName(named, names), namedFileNote(named), namedFileSentence(named)]),
    ).toEqual([
      ['book.rs', null, null],
      ['src/aaa.rs', 'not on the map yet', 'src/aaa.rs is not on the map yet.'],
      ['vendor/gen.rs', 'not on the map', 'vendor/gen.rs is not on the map.'],
      [null, null, null],
    ]);
  });
});
