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
  queries check the time as they work and stop, and the file is reported as timed out. A file
  stored as parsed keeps its rows; any other is stored as read, defining nothing, and is parsed
  again only when it changes.

## Entry points

| Item | What it is |
| --- | --- |
| `discover` | Walks a repository and returns every file, sorted by path, with its language, size, modification time and flags |
| `DiscoverOptions` | The size limit, extra patterns in `.gitignore` syntax, and the user's global gitignore file (the caller finds it from git's configuration) |
| `Discovery`, `Problem` | The files found, the number of links and special files passed over, and what could not be read or used: unreadable files, folders and folder entries, names that cannot be stored, folders past the depth limit, rules files that were skipped, and rules lines that could not be used (the first 20 of a file by number, the rest counted) |
| `Grammars` | Every language's grammar, tags query and imports query, compiled and checked once; a query that uses anything Cortex does not read is refused |
| `Extractor` | Parses one file and reads what it defines, uses and imports; one per thread |
| `Extraction`, `Import` | A file's definitions, with kind, qualified name, enclosing definition, span and hashes; the names it uses, with kind, line and enclosing definition, the names its imports take among them, each with what is written before it (`Qualifier`); and the modules it imports, with what it takes (the module, one name, every name or the default export), the name it binds and whether it only passes what it takes on |
| `index_repository` | Indexes a repository into the store: walks it, removes the stored files it no longer finds, reads, parses and writes every file that changed since the last index, ties references to the definitions their names settle, joins files and definitions in the graph, keeps a node for each folder that holds files, and records the scale mode; reports the definitions whose signature or body changed |
| `IndexOptions`, `IndexReport`, `FileProblem`, `ScipOutcome`, `ScipProblem` | The walk's options, the time limit per file, the number of threads and the indexing time; how many files were written, parsed and removed, what the walk could not do, the files that could not be read, took too long to parse or panicked while being read, and what became of the repository's SCIP index |
| `index_paths` | Indexes the files at chosen paths now, as the walk would find them: writes the new and changed ones, parsed in huge mode too, removes those gone, and ties again only the references the change concerns. The daemon runs it for the paths the watcher reports and for each crawl batch |
| `Crawl` | The order in which huge mode parses the files it took inventory of: files asked for, then the other waiting files of their folders, then the rest, the most recently changed first; each handed out once |
| `watch`, `Watcher`, `Change`, `WatchError` | Watches a repository and reports its changes from a thread of its own: the paths the walk would find that were created, changed, removed or renamed, or a change to everything; stops when the `Watcher` is dropped |

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

`index_repository` writes every file the walk finds that changed since the last index. A file
found as stored, with the same modification time, size, language and flags, is kept without
being read; one whose contents hash as stored gets only its new time. The first index, one
after a Cortex of another version wrote the store, and one after an index or a parse that stopped
partway (the process killed, a write failed) write every file: the rules perhaps changed, or
references were left half tied. An index or a parse marks the store as under way before it writes
and as complete once done.
A written file's definitions keep their ids when their qualified name and kind match the stored
ones, and the report lists those whose signature or body changed.

Each parsed file's row keeps what it passes on of what its top-level imports bring in: each name
it passes on, with the name its import takes and the files the import names, the files whose every
name a `*` import passes on, and whether a `*` import names a module outside the repository. A
name is followed through a file not read in an index by what is kept, as it would be through the
file's imports. What a file's imports bind for its own references is not kept, so some files
around a change are read again for it, without being written: the importers of a changed file that
now passes on something else, and of a removed file, through importers that pass names on, up to
four files away; and the files whose references named like a definition that came or went go
through an import (at most 10,000 files). Their references are tied again. A file added does not
join files whose imports could not name it before until those change or every file is read
again.

A file that may hold secrets, one over the size limit and one that is not text are written as the
walk found them: path, language, size, modification time and flags. Every other file is read whole, and its size, its BLAKE3 hash and
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

First, a SCIP index at the repository's root (`index.scip`), written by the user's or a CI job's own
indexer, ties what it settles at confidence 1.0; Cortex runs no indexer. A symbol the index defines
exactly once, as a namespace, type, term or method, maps to the stored definition with its name
around the definition's line. On each line, the stored references of one name are tied to it when
the index has as many reference occurrences of that name there and all of them are that symbol; a
line where the name also stands for something the index does not know, or for a parameter or local,
ties nothing. Only lines are read from a range, so a document's position encoding does not matter. A
document is left out when its path is not a stored file's or its file changed after the index was
written, and so is every symbol it defines. A reference the index tied before and no longer ties is
tied by the rules below, its file read again for its imports. The index is read one document and one
occurrence at a time; one over 256 MiB, 5 million occurrences or a million distinct symbols, not a
regular file, or not a SCIP index is refused and reported, and the rules below tie as they would
without it.

A bare name, written with nothing before it, may be bound by an import. It is looked up the way
these languages scope names: from the definition the use sits in outwards, through the functions,
methods and modules around it, to the top of the file; a class body is skipped, since its methods
reach its names through `self` or `this`, and the walk stops at a Rust module, which sees nothing
of the scopes around it. The first of these that defines the name, or has an
import that brings it in under that name, decides. A definition there leaves the name to the rules
below; imports there bind it. A bound name is the imports': it is tied through them
(confidence 0.8) when every import binding it there finds the same definition, and stays
unresolved otherwise. What an import finds is the one definition of the name it takes, made
directly in the files it names, in no other definition and with no qualifier (so not a method of
a class or of an `impl` block). A file that does not define the name but passes it on, by name or
with `*`, as a package's `__init__.py` or a TypeScript index file does, is followed, at most four
files deep and with at most 256 looks into files in all; a search that does not finish within
those finds nothing. A file's own definition of the name and its imports of the name must lead to
the same definition, or the name is ambiguous, since which holds can depend on what runs (a
`try` import with a fallback). Its `*` imports make the name ambiguous when they find another
definition, as a later `*` import wins in Python; they decide alone only when nothing else in the
file speaks of the name and none of them names a module outside the repository. A namespace
(`export * as ns`) is not followed. Imports repeated in a file are resolved once.

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
target changed. A reference that goes through an import is stored with the `import_path`
resolution even when its import settles nothing, so a skipped file's references that go through
an import stay as they were, tied or not, since its imports were not read; its other references go
by their names.

Last, the index counts the repository as the scale modes do and records its mode. A source file
is a stored file in a language Cortex reads that is not generated, vendored, binary, over the
size limit or one that may hold secrets; its lines count. More than 20,000 source files or a
million lines make the repository huge; else fewer than 50 files or 5,000 lines make it tiny;
else at most 250 files make it small; else it is medium. A mode set in `IndexOptions::mode`
(`mode` in the configuration) is recorded whatever the size. `IndexReport` gives the size and the
mode, and `mode_for` the mode for any size.

An index in huge mode takes inventory of the files it never parsed instead of reading them: each is
written as the walk found it, with its path, language, size, modification time and flags. Files
parsed before are kept or parsed again as in any index. Huge mode applies when the mode is set to
huge or, left to the size, when the last index recorded huge or the walk finds more than 20,000
source files. Lines are not known before files are read, so a repository huge by its lines alone
is found so by its first index, which reads every file. When an index no longer takes inventory,
it parses every file still waiting. `index_paths` parses the files an agent touches, and `Crawl`
orders the rest, each file and each folder's neighbours queued once however often they are asked
for; how fast to crawl, `index.crawl_files_per_second` in the configuration (100 by default), is
the caller's.

## Indexing chosen paths

`index_paths` indexes the files at the paths it is given, the ones that changed, without walking the
repository. Each path is judged by the walk's own rules: a file the walk would find there is written
unless it is stored as it is, one gone or no longer found is removed, and one that cannot be read
now is left as stored. A path under a folder that is a link, as the walk would never enter it,
counts as gone, and a stored file is read again only when no link has come between the root and the
file. Then the files around the change are read again for what their imports bind, as a full index
does, and only the references the change can concern are tied again: those in the files written and
read again, and those elsewhere tied by name alone whose name's definitions came or went in their
language family, unless the family holds two or more of them both before and after, which leaves
such a reference untied either way. Of the definitions, it reads those in the files it looks into
and, for the name a reference goes by, the first two in the reference's language family: enough to
tell one from several. The SCIP index is not read again: the ties it gave stay, except those into
the files written or removed, which a full index would no longer take from it; the files holding
them are read again, and those references are tied by the rules. The store ends as a full index
would leave it. A path that is or was a folder, a file of rules about which files count that the
walk reads (one in a folder the walk leaves out governs nothing it finds), the SCIP index itself,
and a store whose last index did not finish make it index the whole repository instead.

Each folder that directly holds a stored file gets a graph node, joined to each of those files by
a containment edge that points both ways and carries full strength; how much containment counts
when activation spreads is the containment type's weight, applied where the graph is read. A
folder that holds only folders gets no node, and one whose files are all gone loses its node.

Files are read and parsed in batches on several threads, each thread with a parser of its own,
while the batch before is written in one transaction. Each batch is written in path order, so the
same tree gets the same rows and ids whatever the number of threads, as long as no file runs out
of time.

## Watching

`watch` reports a repository's changes as they happen. File system events are gathered until a path
has been quiet for 200 ms, then sorted by the walk's own rules: paths under `.git` or `.cortex`, and
paths the ignore files and patterns leave out, are dropped. The paths left, created, changed,
removed or renamed (both names), are reported together. More than 500 of them within 2 seconds, as a
branch switch or a large checkout makes, a change to `.gitignore`, `.cortexignore`, `.gitattributes`
or `.git/info` that the walk reads (not one in a folder the walk leaves out, such as an installed
package's), or an error from the file system watcher, is reported as a change to everything instead.
A folder created, removed or renamed may be reported by its own path alone, which stands for
everything under it. Links are not followed, as the walk does not follow them. What to do about a
change, an index of what changed, is the caller's.

## Imports

After the references, each parsed file's imports are turned into the files they name, and the
file gets a directed import edge to each of those files, at weight 0.8. An edge's count is how
many of the file's imports name its target: an import that names one file twice counts once, and
a file gets no edge to itself. A full index replaces the import edges of every file it writes, so
an import that is gone takes its edge with it; a file it skips keeps its edges.

A test file whose imports name no source file, as a Go test beside its package's files, gets an
edge to the file its name spells without its test marks (`orders_test.go` to `orders.go`,
`test_orders.py` to `orders.py`, `cart.test.ts` to `cart.ts`), at weight 0.3, as strong as a tie
by name: the file of that name in the test's own folder, else the only one in the repository, else
none. A test that imports source files is joined to them by its import edges.

Definitions are joined the same way: each definition gets a directed edge to each definition the
references in it are tied to, as calls, uses of a type, inheritance or imports, as strong as the
strongest of those references by how it was tied (0.8, 0.6 or 0.3), and counting them. A
reference at the top of a file, in no definition, gives no edge, and neither does a definition
referring to itself. A full index rewrites these edges from every reference it ties, so a
definition whose references are gone loses its edges.

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
  crate root or module. Inside an inline module (`mod tests { ... }`), `self` is that module and
  the first `super`s climb the inline modules of the file, whose items no file of their own holds,
  so such a path names nothing until it leaves them; a path starting with another name there can
  only be a crate. One that starts with the name of the package's library, or of a package
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
