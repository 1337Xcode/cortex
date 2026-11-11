# cortex-git

A repository's git history as Cortex reads it to learn which files change together. A read walks
the first-parent line from HEAD, newest first, and gives each commit's id, its commit time and the
files it changed against its first parent, with the path a renamed file had before. Two sources
give the same reading: `gix` in-process, and the `git` program, the fallback when `gix` cannot
read a repository. A mining turns a read into counts of which files change together and writes
them to the store, the whole window at first and then only the commits HEAD gains. It also reads
which files the work tree holds changed and not committed, with `git status`, for the daemon to
seed retrievals with. Two more readers serve the reason a span of code looks the way it does:
`git blame` says which commit last changed each line of a span, and a commit's message is read
into a change card, numbers and a kind with the two lines of the message worth showing, which the
caller holds in memory and reads from git again when it needs them.

## What it must never do

- Run a program a repository names, or a `git` the repository could have placed: the `git` program
  is the one the `PATH` names outside the repository. Either source reads, of git's configuration,
  the repository's own file only, and renamed files are compared as git stores them, so no filter
  or text conversion a configuration or an attributes file sets up ever runs. A test proves that
  git itself runs such a conversion for the same history and that neither source does. The work
  tree's reader runs git with the user's own configuration as well, so that it lists what the
  user's git lists, and so never reads a repository whose own configuration names a filter (git
  would run the filter's command on a file it reads to tell whether it changed). It turns off any file system monitor, looks into no submodule (whose own
  configuration could name a filter) and writes no index, so no hook a write of the index would
  start runs. Tests prove that git's own status runs such a filter, wherever the configuration
  names it, a submodule's filter and such a monitor, and writes the index, and that the reader
  does none of these.
- Let git read a file a repository's own configuration includes. Git follows `include.path` and
  `includeIf.<condition>.path` wherever they point, a network share included, and what the file
  holds (a filter, a text conversion, a difference or monitor command) runs. The first question
  asked of git in a repository, before any other run, is whether its own configuration (its file,
  and the work tree's own file where git reads one) holds such a key, asked with includes off; if
  it does, the repository is refused: `OpenError::Includes` for the blamer and the commit reader,
  `WorktreeError::Includes` for the work tree's reader, and an error naming the include for the
  history read by running git. Which file the key names, and whether it exists, makes no
  difference, since it is never read. A test proves that git itself runs a filter and a text
  conversion from an included file, through `include.path`, through `includeIf` on the git
  folder's location and from the work tree's own file, and that no run of this crate does. The
  `gix` source follows no include (its isolated opening turns them off), so a setting only an
  included file holds does not reach it; a test proves that too.
- Read the work tree, the index or an attributes file to read the history. A read of the history
  takes the repository's git folder (its objects, references and own configuration) and nothing
  else, so a damaged index or an attributes file elsewhere changes nothing. Only the work tree's
  reader looks at the work tree, through `git status`. Objects a repository borrows from another one on the
  same machine (git's alternates) are read as git reads them.
- Search above the folder it is given for a repository, or read a repository whose configuration
  names another work tree: paths are relative to the folder Cortex serves.
- Spend unbounded time on one commit. A commit with more deletions or more additions than the cut
  is cut without comparing any file; any other compares at most the cut squared pairs of files.
  The `gix` source compares a file over 1 MiB only by its identity; git's own comparison takes
  time in proportion to the files' sizes.
- Write to the repository, or touch the network. The work tree's reader does not read a partial
  clone with git older than 2.45, which cannot be told not to fetch what the clone left out. In a
  partial clone neither source compares
  contents, which would need what the clone left out, and the `git` program is told not to fetch
  it (`GIT_NO_LAZY_FETCH` and, in a partial clone, `--no-lazy-fetch`; git older than 2.45 knows
  neither and refuses the option, so that source reads no partial clone there).
- Take a broken repository for an empty one: a HEAD that names a tree or a missing object is an
  error; only a HEAD whose branch has no commit yet reads as no history.
- Keep or report a file's contents. Errors name what failed and the commit, nothing more.
- Run a program a repository names, or open a file outside it, when it blames lines or reads a
  commit. `git blame` runs with `--no-textconv` (a text conversion a repository sets up for a
  file would otherwise run, which a test shows), with no system or global configuration, and
  not at all in a repository whose own configuration names a filter. It fetches nothing in a
  partial clone, and a git older than 2.45 is refused there. A repository's own
  `blame.ignoreRevsFile` is opened by git before anything can clear it, so a repository that
  sets it to anything but `.git-blame-ignore-revs` is refused when it is opened. The top
  `.git-blame-ignore-revs` is used only when it is a regular file, not a link, of at most 1 MiB
  holding only comments, blank lines and full object ids, since git fails every blame on an
  abbreviated id. The configuration's mailmap file and blob are replaced by an empty file and
  none, and `blame.showRoot` is pinned off, which decides which commits are boundaries. A blame
  that outlasts the time it is given is killed, with the processes it started on Windows, and
  reported as a timeout, and one that prints more than 1 MiB is refused. The commit reader's two
  runs turn off signature checks (which would run the program `gpg.program` names), notes,
  external diffs and text conversion.
- Keep what a person wrote. A change card holds ids, a time, numbers and a kind. The author's
  and committer's names only decide whether the commit is a bot's and are dropped.

## Entry points

| Item | What it is |
| --- | --- |
| `HistorySource`, `head`, `read` | What a source of history gives: the commit HEAD points to (none before the first commit), and the commits a `Window` covers, newest first, from the commit it names (a mining names the HEAD it resolved, so that a commit made while it reads is the next mining's) or else from HEAD |
| `Window` | What to read: at most `max_commits` commits, none made before `oldest_s` (the walk ends at the first such commit), a commit that changed more than `max_files` files read without its files, and an optional commit to stop at without reading it |
| `Mined`, `MinedCommit`, `PathChange` | What a read gives: each commit's id, time and changed files as repository paths in path order (`None` for a mass commit), whether the stop commit was reached, whether the history is a shallow clone's, and whether the repository is a partial clone |
| `has_repository` | Whether a folder holds a repository at its top (it has `.git`): what both sources and the work tree's reader check first, refusing a folder without one as no repository |
| `GixHistory::committed` | The bytes a file holds in the commit HEAD points to, read from git's object store alone, when it is a regular file within a size limit (its size read from the object's header first); none for a path the commit does not hold, a link, a submodule or a repository with no commit. The daemon reads a changed definition's signature before from it |
| `GixHistory::open` | The source that reads in-process with `gix`. A merge counts against its first parent. A file moved with at least half its contents kept is a rename, git's default threshold; near it the two can differ. In a partial clone, and for files over 1 MiB, only a file moved unchanged is a rename. A shallow clone's cut commits, whose parents are missing, are left out. A path no file can have (not UTF-8, absolute, with a backslash or an empty, `.` or `..` name) is left out |
| `CliHistory::open` | The source that runs the `git` program found on the `PATH` outside the repository, without the caller's `GIT_` variables, for at most 10 s an answer and 300 s and 256 MiB a listing, with every option that shapes its listing on its command line (submodule changes included whatever the repository says). Its listing is read field by field, so a path cannot pass for a commit. It reads what the `gix` source reads, except that git's own rename search decides renames, so an edited file over 1 MiB moved is a rename, and a tree written by hand with a name no file can have makes git refuse the whole listing |
| `cochange::compute`, `cochange::edges`, `cochange::Weighting` | Which files change together, from the commits a read gave and the files that count by their current path (the caller leaves out generated and vendored files): each file's commit count, each pair's co-change count and the newest commit they share, both counts also weighted by recency (`0.5^((anchor - seq) / half-life)` for a commit numbered `seq`: the anchor, the newest commit of a full mining, weighs 1, and commits gained after it more, so that every share is the one a full mining would find); and, from such counts, the edges (`edges`). An edge runs from a file to a partner with at least 2 co-changes and a confidence, the weighted co-changes over the file's weighted commits, of at least 0.1 (a file whose commits all weigh nothing starts none); a file keeps at most 32, ranked by confidence times `ln(1 + co-changes)`, ties by partner. A commit's files are followed to their current paths through newer commits' renames; a mass commit ages older commits but adds nothing. Generic over the file id; deterministic |
| `mine::mine`, `mine::is_due`, `mine::write`, `Mark`, `Mining` | One mining. `mine` starts from where the last one ended, as the store keeps it (`Mark::read`): it reads nothing when HEAD has not moved, only the commits HEAD gained when the commit mined last is still on its first-parent line and they can be added (none renames a file, and the newest lies within 64 half-lives of the anchor), and the whole window when nothing was mined yet, when the last full mining is a week old (so that the window moves with the history) or when the history was rewritten; `is_due` says which without reading. `write` stores it with the store's writer in transactions, letting the caller in between them whenever its `Pause` is due: after a full mining every count and co-change edge is replaced; after commits gained, their counts are added to the stored ones and the edges of the files they changed rewritten from the sums. The mark is removed first and written last, so that a write cut short leaves none and the next mining reads the whole window. Files removed since the read are left out, and each file's partners are ranked among the files left |
| `worktree::uncommitted_files`, `WorktreeError` | The files the work tree holds changed against HEAD, or untracked and not ignored, as `git --no-optional-locks -c core.fsmonitor=false status --porcelain=v1 -z --untracked-files=all --no-renames --ignore-submodules=all -- . ':(exclude).cortex'` lists them in the repository's top, with the user's configuration and without the caller's `GIT_` variables, within 10 s and 4 MiB. A file deleted, staged for deletion or deleted by both sides of a merge is left out; one a merge left in conflict is held, and so is a moved file's new path, never its old one; a path that is not UTF-8 or not a repository path is left out, and so is the top `.gitignore` when its only change since HEAD is the block `cortex init` adds at its end (read from the work tree when it is a file, not a link, of at most 64 KiB, and from HEAD with `git cat-file`; anything that cannot tell leaves it in). A folder without `.git` is `NoRepository`, no git on the `PATH` `NoGit`, a repository whose own configuration names a filter `Refused`, one whose own configuration includes a file `Includes`, and a partial clone with git older than 2.45 `WouldFetch`, all before git lists anything (one question, asked first with includes off, answers whether a file is included; a second, with no system or global configuration and no include followed, answers the filter and the partial clone, and the history's reader asks the same one whether a repository is a partial clone); a listing of more than 4 MiB is `TooMany`, a mass change; a listing that fails or ends late is `Status` |
| `blame::Blamer::open`, `OpenError` | A repository that can be blamed in, made once: it asks git for the repository's own configuration and the git's version one time, and refuses a repository git may not be run in (a filter, a partial clone with a git that would fetch, a blame ignore list other than the top file's) |
| `blame::Blamer::blame`, `BlameRequest`, `BlameError` | The commit that last changed each line of a span of one file at one commit, from one git process, `git blame --porcelain -w -M --no-textconv -L` with a date limit when one is given (`--since`, in git's own seconds-and-zone form; git reads only dates from 100,000,000 on as times, `MIN_SINCE_S`), looking through the commits the request names to ignore (`--ignore-rev`, full ids only: a line one of them changed is blamed on the commit that changed it before, git mapping it through the change), stopped at the request's time. Spans are at most 500 lines (`MAX_BLAME_LINES`). The answer must cover exactly the lines asked for |
| `blame::parse_porcelain`, `Blame`, `BlameRun`, `BlamedCommit`, `Previous` | The parser of porcelain output, which needs no git: runs of lines owned by one commit, each with the line it began at in the commit's own file (lines are one run only when both numbers run on), and for each commit the `previous` commit and path git reports and whether it is a boundary, where the blame stopped and not necessarily where the lines came from (a root commit is one). Details it does not use, such as names and summaries, are skipped as bytes, so none has to be UTF-8. Malformed output is an error |
| `card::CommitReader`, `RECORD_WITHIN`, `COSMETIC_WITHIN` | The two git runs a card needs, through the same runner: `record` (`git show -s --no-show-signature --no-notes --no-color --encoding=UTF-8` with `SHOW_FORMAT`, at most 256 KiB) and `is_cosmetic` (`git show -w --stat` against the first parent, with no textconv, no external diff, no renames and submodules left as they are, which prints nothing for a commit that only moved whitespace), each stopped at its limit: 10 s each (`RECORD_WITHIN`, `COSMETIC_WITHIN`) |
| `card::read_card`, `SHOW_FORMAT`, `CardContext`, `ChangeCard` | A commit's `git show -s` text as a card: id, parents, the committer time (the one a date limit cuts on), the subject (72 characters) and the first body line that is not blank, a trailer, a link, a list of file names, a bare issue reference, a line git wrote or the subject again (100 characters), the pull or merge request number, up to three issues the message closes, whether a bot made it (by the author's name; the committer and a `Signed-off-by` count only when the author's name is empty), and a kind. A kind is `Mass` when the commit changed more paths than the limit (or the miner listed none), else `Bot`, else `Trivial` (a change that only moved whitespace, a `style` type or a `release` or `deps` scope, or a subject that begins with a housekeeping phrase such as `fix typos`, `run rustfmt`, `bump` or `merge branch`, after the `type(scope):` prefix), else `Normal` |
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
file removed between the read and the write, a write cut into turns (the same rows as one turn
gives) and the mark absent while a write is under way. `worktree.rs` reads work trees: a folder
without a repository, a clean work tree, each kind of change (changed, staged, added, untracked in
a new folder, a name with a space, moved, deleted, removed, ignored, and Cortex's own folder), a
filter the repository names in its own file and in the work tree's own file, and one named in a file it includes, in one it includes on its
branch, each refused as an include, a submodule's filter and a file system monitor the
repository names, each run by git's own status and by no look, the index, which git's own status
writes anew and a look leaves as it was, byte for byte, and a partial clone, read with git 2.45
or later and refused with an older one. The porcelain parser, the listing past its bytes, the
configuration's answer (git's booleans among it), git's version and which repositories may be
looked at have unit tests of their own, so the partial clone's refusal is tested whatever git the
tests run with. `includes.rs` makes a repository include a file outside it that names a filter and
a text conversion, through `include.path`, `includeIf` and the work tree's own file, and shows that
git's own status, blame and show run them while the work tree's reader, the blamer, the commit
reader and the history read by running git are each refused and leave no mark; that a harmless or
missing included file is refused as well; that a repository without includes is read as before;
and that `gix` does not read a setting an included file holds. Where a test expects counts and edges,
they are worked out in its comment. `blame.rs` and `card.rs` cover the reasons: a history of five commits (a parser, a fix that closes an issue, a reindentation, a bot's bump and a rename of 36 files) blamed to show that whitespace-only changes and moves within a file keep the commit that wrote a line, that git follows a rename, that each run says where its lines stood in the commit, that `previous` and boundary commits are reported, that a root stays a boundary whatever `blame.showRoot` says, that `.git-blame-ignore-revs` is used only as a regular file of full ids (a link test runs on Unix only), that a repository's own list is refused unless it is that file, that a date limit cuts the blame, that a time of 0 ms ends in a timeout, that more than 1 MiB is refused, that a text conversion, a filter and a fetch in a partial clone never happen (the origin stays up, so a fetch would succeed, and the old blob is still missing afterwards), and that requests which are not blames are refused before git runs; and cards read through `CommitReader` for an ordinary commit and a merge, a configured signature check that must not run, the whitespace check and a message too long to show. The parser, the message patterns, the kinds and their order of precedence have unit tests of their own with every expected value typed by hand.
