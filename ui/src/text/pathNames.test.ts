// Tests for the names files go by: the shortest unshared end of the path, worked out by hand for
// each case below, and the property that no two files of one map share a name.
import { describe, expect, it } from 'vitest';
import { namesOfFiles, pathNames } from './pathNames';

describe('pathNames', () => {
  // README.md is shared by three files and lib.rs by two; main.rs by none.
  const names = pathNames([
    'crates/cortex-store/README.md',
    'ui/README.md',
    'README.md',
    'src/lib.rs',
    'crates/a/src/lib.rs',
    'src/main.rs',
  ]);

  it('keeps a base name no other file shares', () => {
    expect(names.nameOf('src/main.rs')).toBe('main.rs');
  });

  it('adds folders, from the file up, until no other file ends the same way', () => {
    // "README.md" ends three paths; "cortex-store/README.md" ends one.
    expect(names.nameOf('crates/cortex-store/README.md')).toBe('cortex-store/README.md');
    expect(names.nameOf('ui/README.md')).toBe('ui/README.md');
    // "src/lib.rs" is the whole of another path, so one more folder: "a/src/lib.rs".
    expect(names.nameOf('crates/a/src/lib.rs')).toBe('a/src/lib.rs');
  });

  it('names a path from the root when every end of it ends a longer path too', () => {
    expect(names.nameOf('README.md')).toBe('./README.md');
    // "lib.rs" and "src/lib.rs" both end "crates/a/src/lib.rs".
    expect(names.nameOf('src/lib.rs')).toBe('./src/lib.rs');
  });

  it('names a path the map does not hold in full', () => {
    expect(names.nameOf('docs/new.md')).toBe('docs/new.md');
    expect(names.nameOf('main.rs')).toBe('main.rs');
  });

  it('compares whole folder names, never the end of one', () => {
    // "ab/c.rs" does not end with the folder "b": "b/c.rs" is unshared.
    const split = pathNames(['ab/c.rs', 'b/c.rs']);
    expect(split.nameOf('b/c.rs')).toBe('b/c.rs');
    expect(split.nameOf('ab/c.rs')).toBe('ab/c.rs');
  });

  it('counts a path listed twice as one file', () => {
    expect(pathNames(['a/x.rs', 'a/x.rs']).nameOf('a/x.rs')).toBe('x.rs');
  });

  it('gives every file of a map its own name', () => {
    const paths = [
      'mod.rs',
      'a/mod.rs',
      'b/a/mod.rs',
      'c/b/a/mod.rs',
      'x/mod.rs',
      'y/x/mod.rs',
      'src/orders/book.rs',
      'tests/orders/book.rs',
      'book.rs',
    ];
    const map = pathNames(paths);
    const shown = paths.map((path) => map.nameOf(path));
    expect(new Set(shown).size).toBe(paths.length);
    // Each name is an end of its path, in whole segments, or the path from the root.
    for (const [index, path] of paths.entries()) {
      const name = shown[index] ?? '';
      expect(name === `./${path}` || path === name || path.endsWith(`/${name}`)).toBe(true);
    }
  });
});

describe('namesOfFiles', () => {
  it('gives one set of names per list of files', () => {
    const files = [{ path: 'a/mod.rs' }, { path: 'b/mod.rs' }];
    expect(namesOfFiles(files)).toBe(namesOfFiles(files));
    expect(namesOfFiles(files).nameOf('b/mod.rs')).toBe('b/mod.rs');
  });
});
