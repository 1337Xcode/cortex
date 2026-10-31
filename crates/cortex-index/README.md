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
| `Extraction`, `Import` | A file's definitions, with kind, qualified name, enclosing definition, span and hashes; the names it uses, with kind, line and enclosing definition, the names its imports take among them, each with what is written before it (`Qualifier`); and the modules it imports, with what it takes (the module, one name, every name or the default export), the name it binds and whether it only passes what it takes on |
| `index_repository` | Indexes a whole repository into the store: walks it, removes the stored files it no longer finds, reads, parses and writes every file it found, ties references to the definitions their names settle, joins each parsed file to the files its imports name, and keeps a node for each folder that holds files |
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
imports, in the order they are written. Each name an import takes is also a reference, of kind
`import`, at the name itself: a name on its own line in an import that spans several lines is
on that line, and one imported inside a function sits in that function.

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

Each reference is then tied to a definition when what its file says settles which one.

A bare name, written with nothing before it, may be bound by an import. It is looked up the way
these languages scope names: from the definition the use sits in outwards, through the functions,
methods and modules around it, to the top of the file; a class body is skipped, since its methods
reach its names through `self` or `this`. The first of these that defines the name, or has an
import that brings it in under that name, decides. A definition there leaves the name to the rules
below; imports there bind it. A bound name is the imports': it is tied through them
(confidence 0.8) when every import binding it there finds the same definition, and stays
unresolved otherwise. What an import finds is the one definition of the name it takes, made
directly in the files it names, in no other definition and with no qualifier (so not a method of
a class or of an `impl` block). A file that does not define the name but passes it on, by name or
with `*`, as a package's `__init__.py` or a TypeScript index file does, is followed, at most four
files deep and through at most 256 files in all; a search that does not finish within those finds
nothing. A namespace (`export * as ns`) is not followed.

Some imports bind no bare name. A re-export (`export { x } from`, `export * from`) only passes
names on. An import of every name (`from x import *`, Go's dot import) does too, since which names
it brings in depends on each language's rules of visibility and order. A default import binds
nothing, since no definition says it is the default. Parameters and local variables are not
known, so one named like an import does not hide it.

A name written after something, as in `line.strip()` or `orders.Place()`, is never bound by an
import of the bare name; each language whose imports name files lists the forms of its grammar
that write a name after something, beside its parsing rules. When what is written before the name
is a plain name, that name is looked up through the same scopes. If the imports there give it a
module, the name is that module's: it is tied (0.8) when they all find the same definition, made
in the module's files or passed on by them, and stays unresolved otherwise. These imports give a
module: Go's, by the package's alias or the last element of its path (the one before a major
version such as `v2`; the blank name `_` reaches nothing); a namespace import
(`import * as api`); Python's `import shop.models`, as `shop.models` or its alias, and
`from shop import models` when `models` is a submodule that `shop` does not define as something
else; Rust's `use crate::models` of a module file, and a path brought in whole. If they give it
something outside the repository, as `http.Error` or `os.path.join` do, the name stays
unresolved. If an import gives it a definition of the repository, a class or a value, the name
goes by its own name, since only that definition's members could make it.

An import's own reference is tied through its own import or not at all. Any other reference that
no import binds is tied to the one definition with its name in its own file (0.6), or else to the
one definition with its name among the files of its language family (0.3). The families are
TypeScript, TSX and JavaScript; C and C++; Java, Kotlin and Scala; every other language is a family
of its own, so a call is never tied to a function of another language that shares its name. A name
with several definitions in reach stays unresolved. Every full index resolves every stored
reference again, the kept references of skipped files included, and writes only the ones whose
target changed; a skipped file's import references and its ties through imports stay as they
were, since its imports were not read.

Each folder that directly holds a stored file gets a graph node, joined to each of those files by
a containment edge that points both ways and carries full strength; how much containment counts
when activation spreads is the containment type's weight, applied where the graph is read. A
folder that holds only folders gets no node, and one whose files are all gone loses its node.

Files are read and parsed in batches on several threads, each thread with a parser of its own,
while the batch before is written in one transaction. Each batch is written in path order, so the
same tree gets the same rows and ids whatever the number of threads, as long as no file runs out
of time.

## Imports

After the references, each parsed file's imports are turned into the files they name, and the
file gets a directed import edge to each of those files, at weight 0.8. An edge's count is how
many of the file's imports name its target: an import that names one file twice counts once, and
a file gets no edge to itself. A full index replaces the import edges of every file it writes, so
an import that is gone takes its edge with it; a file it skips keeps its edges.

Each language family whose imports can name files has a resolver in `src/imports/`, registered by
one line there. A resolver reads the project files it needs (configs, `go.mod`, `Cargo.toml`)
once per index, as the walk found them, and never one that may hold secrets.

- TypeScript and JavaScript. A specifier that starts with `./` or `../` is taken from the
  importer's folder. Any other goes through the nearest `tsconfig.json` or `jsconfig.json` above
  the importer, `tsconfig.json` first: an exact `paths` pattern, else the longest matching
  prefix, else `baseUrl`. A config's relative `extends` is followed, with `.json` added when the
  path names no file; a config from a package is not read. Each path is tried the way TypeScript
  tries it: a script extension written in the specifier is swapped for the ones TypeScript would
  load in its place, then `.ts`, `.tsx`, `.d.ts`, `.js` and `.jsx` are added, then the folder's
  `index` file is tried with each. A package name with no mapping names no file.
- Python. A relative import climbs one folder per dot after the first, through packages only
  (folders with `__init__.py`). An absolute import is looked for under the importer's base
  folder (the nearest folder above it that is not a package), then under the repository root
  when that is not a package, then under the one source root that holds it. Source roots are the
  folders holding `pyproject.toml`, `setup.py` or `setup.cfg`, and the `src` folder beside them.
  `from x import y` also names `x/y.py` when `x` is a package.
- Go. The import path is matched against the `module` lines of the repository's `go.mod` files,
  longest module first, and the rest of the path names a folder under that module. The package's
  `.go` files are the import's files, test files and files whose names start with `_` or `.` left
  out. When two `go.mod` files declare the same module, the one in a folder above the importer is
  used.
- Rust. A path that starts with `crate`, `self` or `super` is walked from the importer's own
  crate root or module. One that starts with the name of the package's library, or of a package
  of the repository it depends on by `path` (directly or through its workspace), is walked from
  that crate's library root. Each part is looked for as `x.rs`, then `x/mod.rs`. Crate roots are
  Cargo's targets: the library, `src/main.rs`, the build script, and the binaries, tests,
  examples and benchmarks, declared or found where Cargo finds them.

Other languages' imports name no file. Every limit on this work is fixed, so a crafted
repository cannot make it grow: a config built from more than 16 files (itself and everything it
extends), or with more than 256 wildcard `paths` patterns or more than 16 targets for one
pattern, is not used, and a Python top-level name held by more than 64 source roots is left
unresolved. A config that is not used, or that extends a file that cannot be read, leaves only
relative specifiers to resolve.

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
