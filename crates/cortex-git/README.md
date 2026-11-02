# cortex-git

A repository's git history as Cortex reads it to learn which files change together. A read walks
the first-parent line from HEAD, newest first, and gives each commit's id, its commit time and the
files it changed against its first parent, with the path a renamed file had before.

## What it must never do

- Run a program a repository names. The repository is opened isolated: of git's configuration only
  the repository's own file is read. Renamed files are compared as git stores them, with no
  attributes, so no filter or text conversion a configuration or an attributes file sets up ever
  runs. A test proves that git itself runs such a conversion for the same history and that a read
  does not.
- Read the work tree, the index or an attributes file. A read takes the repository's git folder
  (its objects, references and own configuration) and nothing else, so a damaged index or an
  attributes file elsewhere changes nothing. Objects a repository borrows from another one on the
  same machine (git's alternates) are read as git reads them.
- Search above the folder it is given for a repository, or read a repository whose configuration
  names another work tree: paths are relative to the folder Cortex serves.
- Spend unbounded time on one commit. A commit with more deletions or more additions than the cut
  is cut without comparing any file; any other compares at most the cut squared pairs of files,
  and a file over 1 MiB only by its identity.
- Write to the repository, or touch the network.
- Keep or report a file's contents. Errors name what failed and the commit, nothing more.

## Entry points

| Item | What it is |
| --- | --- |
| `HistorySource`, `head`, `read` | What a source of history gives: the commit HEAD points to (none before the first commit), and the commits a `Window` covers, newest first |
| `Window` | What to read: at most `max_commits` commits, none made before `oldest_s` (the walk ends at the first such commit), a commit that changed more than `max_files` files read without its files, and an optional commit to stop at without reading it |
| `Mined`, `MinedCommit`, `PathChange` | What a read gives: each commit's id, time and changed files as repository paths in path order (`None` for a mass commit), whether the stop commit was reached, whether the history is a shallow clone's, and whether the repository is a partial clone |
| `GixHistory::open` | The source that reads in-process with `gix`. A merge counts against its first parent. A file moved with at least half its contents kept is a rename, git's default threshold; near it the two can differ. In a partial clone, and for files over 1 MiB, only a file moved unchanged is a rename. A shallow clone's cut commits, whose parents are missing, are left out. A path no file can have (not UTF-8, absolute, with a backslash or an empty, `.` or `..` name) is left out |
| `GitError` | Why a read failed: no repository at the folder's top, one that cannot be opened, HEAD, the walk or one commit's changes, each with its causes |

## Tests

```sh
cargo test -p cortex-git
```

Each test makes a repository with the `git` program in a temporary folder, commit by commit at
fixed times, and compares the read with the commits and paths written by hand, the ids taken from
git itself. `gix_history.rs` covers the walk: a linear history, a merge, the window's count and
time limits, mass commits cut by files changed before and new, the stop commit, a shallow clone,
an empty repository, a detached HEAD, a damaged index, and folders that are not a repository's top.
`renames.rs` covers renames: kept and rewritten files, as many renames as the cut allows, a mass
commit whose deleted contents are gone from the repository (read without comparing them), large
files, a partial clone, attributes that git ignores for renames too, a text conversion that must
not run, a tree written by hand with names no file can have, and on Unix a path that is not UTF-8.
