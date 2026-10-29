# cortex-index

The indexer: which files a repository has and what they are.

## What it must never do

- Open a file whose name, or the name of a folder above it, says it may hold secrets (`.env*`,
  `*.pem`, `*.key`, `id_rsa*`, `*credentials*`, `*.p12`, in any case), rules files such as
  `.gitignore` in those folders included. Such a file is recorded by path and size only.
- Read a file larger than `index.max_file_bytes`, or anything but a regular file. A path is
  inspected without following links, opened, and checked again after opening; symbolic links and
  special files found in the walk are passed over and counted.
- Enter `.git` or `.cortex`, go more than 256 folders below the root, or stop for one unreadable
  file or folder. Problems are reported beside the results.

## Entry points

| Item | What it is |
| --- | --- |
| `discover` | Walks a repository and returns every file, sorted by path, with its size, modification time and flags |
| `DiscoverOptions` | The size limit, extra patterns in `.gitignore` syntax, and the user's global gitignore file (the caller finds it from git's configuration) |
| `Discovery`, `Problem` | The files found, the number of links and special files passed over, and what could not be read or used: unreadable files, folders and folder entries, names that cannot be stored, folders past the depth limit, rules files that were skipped, and rules lines that could not be used (the first 20 of a file by number, the rest counted) |

## What a file's flags mean

| Flag | When |
| --- | --- |
| `SECRET_PATTERN` | Its name or a folder's matches a secret pattern; it was never opened |
| `TOO_LARGE` | It is larger than the limit; it was never read |
| `BINARY` | A NUL byte appears in its first 8 KiB |
| `GENERATED` | It is in a `dist`, `build`, `target`, `.next`, `out` or `coverage` folder, is a minified script, source map or package lockfile, or a comment in its first 40 lines carries a code generator's marker; or `.gitattributes` sets `linguist-generated` for it |
| `VENDORED` | It is in a `node_modules` or `vendor` folder, or `.gitattributes` sets `linguist-vendored` for it |

## Rules

The walk honours `.gitignore` and `.cortexignore` files in every folder, `.git/info/exclude`, the
global gitignore (its patterns relative to the repository root) and `index.extra_ignores`, with
git's precedence: the extra patterns first, then the deepest folder up, `.cortexignore` before
`.gitignore`, then `.git/info/exclude`, then the global file. It includes hidden files.

`.gitattributes` files in every folder and `.git/info/attributes` are read with git's precedence,
and `-linguist-generated` or `-linguist-vendored` clears a flag the path would give. Each
attributes file's patterns are compiled into one set, so its size does not multiply the cost of
the walk.

As in git, a byte order mark before a rules file's first line is ignored and braces in patterns
are literal. Matching is case-sensitive on every platform.

## Tests

```sh
cargo test -p cortex-index
```

The tests build folder trees in temporary directories and list the expected files, flags and
problems by hand. Files that must never be opened are locked against reading during the test,
next to locked ordinary files that show the lock works. Cases that only Unix can create (names
that are not UTF-8, unreadable folders, symbolic links, named pipes) run on Unix.
