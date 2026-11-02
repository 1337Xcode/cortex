# cortex-git

A repository's git history as Cortex reads it to learn which files change together. A read walks
the first-parent line from HEAD, newest first, and gives each commit's id, its commit time and the
files it changed against its first parent, with the path a renamed file had before. Two sources
give the same reading: `gix` in-process, and the `git` program, the fallback when `gix` cannot
read a repository. A mining turns a read into counts of which files change together and writes
them to the store, the whole window at first and then only the commits HEAD gains.

## What it must never do

- Run a program a repository names, or a `git` the repository could have placed: the `git` program
  is the one the `PATH` names outside the repository. Either source reads, of git's configuration,
  the repository's own file only, and renamed files are compared as git stores them, so no filter
  or text conversion a configuration or an attributes file sets up ever runs. A test proves that
  git itself runs such a conversion for the same history and that neither source does.
- Read the work tree, the index or an attributes file. A read takes the repository's git folder
  (its objects, references and own configuration) and nothing else, so a damaged index or an
  attributes file elsewhere changes nothing. Objects a repository borrows from another one on the
  same machine (git's alternates) are read as git reads them.
- Search above the folder it is given for a repository, or read a repository whose configuration
  names another work tree: paths are relative to the folder Cortex serves.
- Spend unbounded time on one commit. A commit with more deletions or more additions than the cut
  is cut without comparing any file; any other compares at most the cut squared pairs of files.
  The `gix` source compares a file over 1 MiB only by its identity; git's own comparison takes
  time in proportion to the files' sizes.
- Write to the repository, or touch the network. In a partial clone neither source compares
  contents, which would need what the clone left out, and the `git` program is told not to fetch
  it (`GIT_NO_LAZY_FETCH` and, in a partial clone, `--no-lazy-fetch`; git older than 2.45 knows
  neither and refuses the option, so that source reads no partial clone there).
- Take a broken repository for an empty one: a HEAD that names a tree or a missing object is an
  error; only a HEAD whose branch has no commit yet reads as no history.
- Keep or report a file's contents. Errors name what failed and the commit, nothing more.

## Entry points

| Item | What it is |
| --- | --- |
| `HistorySource`, `head`, `read` | What a source of history gives: the commit HEAD points to (none before the first commit), and the commits a `Window` covers, newest first |
| `Window` | What to read: at most `max_commits` commits, none made before `oldest_s` (the walk ends at the first such commit), a commit that changed more than `max_files` files read without its files, and an optional commit to stop at without reading it |
| `Mined`, `MinedCommit`, `PathChange` | What a read gives: each commit's id, time and changed files as repository paths in path order (`None` for a mass commit), whether the stop commit was reached, whether the history is a shallow clone's, and whether the repository is a partial clone |
| `GixHistory::open` | The source that reads in-process with `gix`. A merge counts against its first parent. A file moved with at least half its contents kept is a rename, git's default threshold; near it the two can differ. In a partial clone, and for files over 1 MiB, only a file moved unchanged is a rename. A shallow clone's cut commits, whose parents are missing, are left out. A path no file can have (not UTF-8, absolute, with a backslash or an empty, `.` or `..` name) is left out |
| `CliHistory::open` | The source that runs the `git` program found on the `PATH` outside the repository, without the caller's `GIT_` variables, for at most 10 s an answer and 300 s and 256 MiB a listing, with every option that shapes its listing on its command line (submodule changes included whatever the repository says). Its listing is read field by field, so a path cannot pass for a commit. It reads what the `gix` source reads, except that git's own rename search decides renames, so an edited file over 1 MiB moved is a rename, and a tree written by hand with a name no file can have makes git refuse the whole listing |
| `cochange::compute` | Which files change together, from the commits a read gave and the files that count by their current path (the caller leaves out generated and vendored files): each file's commit count, each pair's co-change count and the newest commit they share, both counts also weighted by recency (`0.5^(age / half-life)`, age in commits from the newest), and the edges. An edge runs from a file to a partner with at least 2 co-changes and a confidence, the weighted co-changes over the file's weighted commits, of at least 0.1; a file keeps at most 32, ranked by confidence times `ln(1 + co-changes)`, ties by partner. A commit's files are followed to their current paths through newer commits' renames; a mass commit ages older commits but adds nothing. Generic over the file id; deterministic |
| `mine::mine`, `mine::write`, `Mark`, `Mining` | One mining. `mine` starts from where the last one ended, as the store keeps it (`Mark::read`): it reads nothing when HEAD has not moved, only the commits HEAD gained when the commit mined last is still on its first-parent line, and the whole window when nothing was mined yet, when the last full mining is a week old (so that recency weights catch up with the history) or when the history was rewritten. `write`, in a transaction of the store's writer, replaces every count and co-change edge after a full mining; after commits gained, it adds their counts to the stored ones and rewrites the edges of the files they changed from the sums. Files removed since the read are left out |
| `GitError` | Why a read failed: no repository at the folder's top, one that cannot be opened, HEAD, the walk or one commit's changes, each with its causes |

## Tests

```sh
cargo test -p cortex-git
```

Each test makes a repository with the `git` program in a temporary folder, commit by commit at
fixed times, and compares what both sources read with the commits and paths written by hand, the
ids taken from git itself. `history.rs` covers the walk: a linear history (with the repository's
own log settings set to change what git shows), a merge, the window's count and time limits, a
walk ending at the first older commit when commit times are out of order, mass commits cut by
files changed before and new, the stop commit (also right after the window's last commit), a
shallow clone, a stop commit older than the window, an empty repository, a detached HEAD, a HEAD
naming a tree or a missing object, a damaged index, a submodule moved with the repository set to
ignore submodules, and folders that are not a repository's top. `renames.rs` covers renames: kept and rewritten files, as many renames as the
cut allows, a mass commit whose deleted contents are gone from the repository (read without
comparing them), large files, a partial clone whose origin is gone, attributes that git ignores
for renames too, a text conversion that must not run, a tree written by hand with names no file
can have, and on Unix a path that is not UTF-8. The listing's parser has unit tests of its own.
`cochange.rs` works small histories out by hand, the arithmetic in each test's comment: counts,
weights and confidence, renames followed (also two in one commit), files that do not count, a
mass commit's age, the edge thresholds, the partner cap and its ranking; and a property test
over random histories (deterministic output, pair counts within their files', edges within the
thresholds, at most 32 a file, everything in order). `mine.rs` mines into a store: the first
mining, commits gained (and HEAD unmoved, which writes nothing), a rewritten history, a file's
edges left alone when commits gained did not change it, a full mining again a week later, and a
file removed between the read and the write. Where a test expects counts and edges, they are
worked out in its comment.
