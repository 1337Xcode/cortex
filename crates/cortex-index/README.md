# cortex-index

The indexer: which files a repository has, what they are, and what they define and use.

## What it must never do

- Open a file whose name, or the name of a folder above it, says it may hold secrets (`.env*`,
  `*.pem`, `*.key`, `id_rsa*`, `*credentials*`, `*.p12`, in any case), rules files such as
  `.gitignore` in those folders included. Such a file is recorded with what its folder listing
  and its name say: path, size, modification time, flags and the language its name implies.
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
| `index_repository` | Indexes a whole repository into the store: walks it, removes the stored files it no longer finds, reads, parses and writes every file it found, ties references to the definitions their names settle, and keeps a node for each folder that holds files |
| `IndexOptions`, `IndexReport`, `FileProblem` | The walk's options, the time limit per file, the number of threads and the indexing time; how many files were written, parsed and removed, what the walk could not do, and the files skipped because they could not be read, took too long to parse or panicked while being read |

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

Each file is parsed once with tree-sitter. Its tags query, the grammar's own and Cortex's
patterns in `queries/` in the order its language file gives, finds definitions and references in
tree-sitter's tags format: each pattern captures one `@name` and one `@definition.<kind>` or
`@reference.<kind>`. Where two patterns find the same name, the earlier one wins, and a name
holding a syntax error is skipped. A pattern that captures `@ignore` keeps later patterns from
recording that name, as Go's built-in types are kept out, and one that captures a `@name` alone
records nothing. Locals patterns find scopes (`@local.scope`) and the variables defined in them
(`@local.definition`), and a pattern marked `#is-not? local` skips a name that a scope around it
defined earlier: that keeps a Ruby local variable from being read as a method call. Comments a
pattern captures as `@doc` are ignored, since Cortex keeps no comments, and a capture named with
a leading `_` only serves a text predicate such as `#eq?`. A second query per language finds
imports.

Definitions nest by their spans. A function directly in a class is a method; in a test file (one
in a `test`, `tests` or `__tests__` folder, or named as its language names test files) a
function or method named as its language names tests is a test. Qualified names join names with
`.` in every language. A pattern may capture a `@qualifier` for a definition, which goes just
before its name: a Go method's qualified name starts with the type it belongs to. A name written
across several tokens, such as `Shop.Orders` or `operator <`, is read token by token: comments,
line breaks and spacing between the tokens are not part of it, except one space where two words
meet (`operator new`). Import specifiers are read the same way.

A decorated definition starts at its first decorator, and every definition ends at its last
character that is not whitespace, though its node may end with a line break, as a C `#define`
does. A definition carries two hashes: of its header, from its start to the last part before its
body, and of all of it. Comments between the header and the body count only in the second, and
each run of whitespace counts as one space, so reindenting or rewrapping a definition keeps both
hashes and editing its body changes only the second.

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

## Indexing a repository

`index_repository` writes every file the walk finds. A file that may hold secrets, one over the
size limit and one that is not text are written as the walk found them: path, language, size,
modification time and flags. Every other file is read whole, and its size, its BLAKE3 hash and
its line count (one per line feed, and one more for text after the last) are recorded. It is
parsed when Cortex parses its language and it is neither generated nor vendored; its definitions
and references are written with it, and it gets the `PARSED` flag.

A name a file's queries find but cannot keep is left out and counted: one a syntax error
touches, alone, the definitions inside it recorded as if it were not there; one that is not a
name Cortex can store, with the definitions inside it, whose qualified names would hold it. The
report counts, per file, the definitions, references and imports left out. Names a query ignores
on purpose are not counted.

A file whose reading or parsing fails is skipped: it is reported and not written, and what the
store held for it stays, or nothing when it held nothing. Reading fails when the file cannot be
read as the walk found it, because it is gone, locked or has grown past the limit since; parsing
fails when it takes longer than the time limit. A panic while a file is read or parsed, a fault
in Cortex or in a grammar, is caught and skips that file only.

A stored file the walk no longer finds is removed, unless the walk reported a problem at its path
or at a folder above it: a folder it could not list or did not enter, or a file it could not
inspect.

Each reference is then tied to the one definition with its name in its own file (confidence 0.6),
or else to the one definition with its name among the files of its language family (0.3). The
families are TypeScript, TSX and JavaScript; C and C++; Java, Kotlin and Scala; every other
language is a family of its own, so a call is never tied to a function of another language that
shares its name. A name with several definitions in reach stays unresolved. Every full index
resolves every stored reference again, the kept references of skipped files included, and
writes only the ones whose target changed.

Each folder that directly holds a stored file gets a graph node, joined to each of those files by
a containment edge that points both ways and carries full strength; how much containment counts
when activation spreads is the containment type's weight, applied where the graph is read. A
folder that holds only folders gets no node, and one whose files are all gone loses its node.

Files are read and parsed in batches on several threads, each thread with a parser of its own,
while the batch before is written in one transaction. Each batch is written in path order, so the
same tree gets the same rows and ids whatever the number of threads, as long as no file runs out
of time.

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

A scan indexes every sample with the real pipeline and then searches the database file and its
log, byte for byte, for each line of the samples of 12 characters or more. No line may be found
there unless an empty store already holds it (its own schema text) or it is a name by design (a
Markdown heading's text).
