# cortex-index

The indexer: which files a repository has, what they are, and what they define and use.

## What it must never do

- Open a file whose name, or the name of a folder above it, says it may hold secrets (`.env*`,
  `*.pem`, `*.key`, `id_rsa*`, `*credentials*`, `*.p12`, in any case), rules files such as
  `.gitignore` in those folders included. Such a file is recorded by path and size only.
- Read a file larger than `index.max_file_bytes`, or anything but a regular file. A path is
  inspected without following links, opened, and checked again after opening; symbolic links and
  special files found in the walk are passed over and counted.
- Enter `.git` or `.cortex`, go more than 256 folders below the root, or stop for one unreadable
  file or folder. Problems are reported beside the results.
- Take a name from anything but the name a query captures for it, or take as a name the whole
  text of a definition or an expression. A name is one line of at most 256 bytes; a definition
  whose name or qualified name is longer is left out, with everything inside it.
- Go on parsing or querying a file once it has had `index.parse_timeout_ms`. The parser and the
  queries check the time as they work and stop, and the file is reported as timed out.

## Entry points

| Item | What it is |
| --- | --- |
| `discover` | Walks a repository and returns every file, sorted by path, with its language, size, modification time and flags |
| `DiscoverOptions` | The size limit, extra patterns in `.gitignore` syntax, and the user's global gitignore file (the caller finds it from git's configuration) |
| `Discovery`, `Problem` | The files found, the number of links and special files passed over, and what could not be read or used: unreadable files, folders and folder entries, names that cannot be stored, folders past the depth limit, rules files that were skipped, and rules lines that could not be used (the first 20 of a file by number, the rest counted) |
| `Grammars` | Every language's grammar, tags query and imports query, compiled and checked once; a query that uses anything Cortex does not read is refused |
| `Extractor` | Parses one file and reads what it defines, uses and imports; one per thread |
| `Extraction`, `Import` | A file's definitions, with kind, qualified name, enclosing definition, span and hashes; the names it uses, with kind, line and enclosing definition; and the modules it imports, with what it takes (the module, one name, every name or the default export) and the name it binds |

## What a file's flags mean

| Flag | When |
| --- | --- |
| `SECRET_PATTERN` | Its name or a folder's matches a secret pattern; it was never opened |
| `TOO_LARGE` | It is larger than the limit; it was never read |
| `BINARY` | A NUL byte appears in its first 8 KiB |
| `GENERATED` | It is in a `dist`, `build`, `target`, `.next`, `out` or `coverage` folder, is a minified script, source map or package lockfile, or a comment in its first 40 lines carries a code generator's marker; or `.gitattributes` sets `linguist-generated` for it |
| `VENDORED` | It is in a `node_modules` or `vendor` folder, or `.gitattributes` sets `linguist-vendored` for it |

## Languages

A file's language comes from `linguist-language` in `.gitattributes` when that names one (a
language Cortex does not parse leaves the file without one), otherwise from its whole name
(`Gemfile`, `.bashrc`), its extension, or the interpreter its `#!` line names. A `.h` header is
C++ when its start holds something only C++ has, and C otherwise. Each language has its own file
in `src/languages/`; adding one means adding a file and a line in the registry.

## Definitions, references and imports

Each file is parsed once with tree-sitter. Its tags query, the grammar's own followed by
Cortex's additions in `queries/`, gives definitions and references in tree-sitter's tags format:
each pattern captures one `@name` and one `@definition.<kind>` or `@reference.<kind>`. Where two
patterns find the same name, the earlier one wins, and a name holding a syntax error is skipped.
Comments a pattern captures as `@doc` are ignored, since Cortex keeps no comments, and a capture
named with a leading `_` only serves a text predicate such as `#eq?`. A second query per
language finds imports.

Definitions nest by their spans. A function directly in a class is a method; in a test file (one
in a `test`, `tests` or `__tests__` folder, or named as its language names test files) a
function or method named as its language names tests is a test. Qualified names join names with
`.` in every language.

A decorated definition starts at its first decorator. A definition carries two hashes: of its
header, from its start to the last part before its body, and of all of it. Comments between the
header and the body count only in the second, and each run of whitespace counts as one space, so
reindenting or rewrapping a definition keeps both hashes and editing its body changes only the
second.

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

Each parsed language has a sample file in `tests/languages/<language>/` beside the definitions,
references and imports expected from it, written by hand.
