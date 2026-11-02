# cortex-learn

What Cortex learns from agents' work. For now, the record of it: each normalized hook event becomes
the session, task and event rows the store keeps, each edit only the file watcher saw is given to
the session most likely to have made it, or to none, and what each open task did is kept until it
ends. It also tells what a shell command line ran: the checks of the runners it knows, the user's
own test commands, and git restores, what became of a task that has ended, what its work adds to the interest of each node it touched, and the links it draws between the nodes it used together.

## What it must never do

- Keep a prompt's text (a task keeps the prompt's hash, and its first 200 characters only when
  `privacy.store_prompt_excerpt` is on), a command, a search pattern or anything a tool printed.
  The words that say why a task ended as it did name what a check vouched for, the runner and its
  exit code, never the command line, a path or code.
- Call a task a success without having seen a check pass on its work, or a revert without every
  file it edited being back as it was.
- Let a score or a weight stand for more than it was earned at: each is kept as of the task that
  earned it, and the decay of the tasks since is applied where it is read.
- Teach anything from a task that failed, was reverted or was left: links grow from work that was
  seen to hold up, and the files a reverted task changed together lose what they gained.
- Write the store itself: it records inside a transaction the daemon's single writer opens.
- Take a check as passed when the command line's exit status does not vouch for it:
  `cargo test | tail` exits as `tail` does, so it tells nothing.

## Entry points

| Item | What it is |
| --- | --- |
| `Recorder::new`, `Recorder::record` | Records one event in a write transaction, the shell commands told by the `Commands` it is given. A prompt starts a task numbered across the repository, ending first any task its session left open when the end of the previous answer never arrived; the end of the answer or of the session ends it as `unknown`. A subagent is a session of its own with its parent's id, and its events belong to the parent's open task. A tool call is recorded after it ran, as a read, a search, an edit (one row per changed stretch of lines, up to `cortex_core::MAX_RANGES`), a shell command (with the runner and kind of the check its exit status vouches for, by `Commands`; the highest ranked when it ran several: tests, a build, a type check, a lint) or another tool, with the file's graph node when the file is indexed; the moment before a tool runs is not kept. An event from a session the store has not seen registers the session. `record` returns the `TaskState` of the task the event ended, if any. After a transaction that was rolled back, `Recorder::forget_stored` drops what the recorder took the store to hold, the tasks' states included, which are rebuilt from the store; what it saw of sessions' work stays |
| `TaskState`, `Step`, `FileNode`, `RunCheck`, `MAX_TASK_ROWS`, `MAX_TASK_STATES`, `MAX_TASK_HIT_BYTES` | What a task did, kept as its rows are stored: each read, edit, search and shell command a step in the order recorded (a read, edit or search whose tool reported a failure left out), with when it happened, its file and graph node, its lines, the check of a shell command (its runner and kind) and its exit code. While the daemon runs, a search's step also holds the files it found (a task's searches at most 64 KiB of their paths, the first found first) and a shell command's the git restores it ran when it exited 0; and `before` gives what each file the task edited held before the task first changed it (no file, a content hash, or contents not read), from whichever came first, the index's report of the change or the store before it; the rows keep none of these. A state not held, after a restart or once dropped for room (at most 32 are held, the one touched longest ago dropped first), is rebuilt from the task's rows when the task is next touched. A task of more than 4,096 rows keeps the steps of its first 4,096 and is marked partial, as is a state rebuilt from rows that cannot all be read back, which holds none of them. When the task ends, each file without a node is given the one the store has for its path then, and the state carries how the task ended: the answer to it ended, or it was left |
| `Recorder::hold_watcher_edits`, `Recorder::next_watcher_edit_due_ms`, `Recorder::take_due_watcher_edits`, `HeldEdit`, `Recorder::record_watcher_edit`, `WatcherEdit`, `HOLD_MS`, `ATTRIBUTION_WINDOW_MS` | Edits the file watcher saw, each held 2 s from when it was first seen, with what its file held before as the index found it; a hook's edit of the file within those 2 s takes that content for its task. Then a hook's edit of the same file at most 2 s before or after it claims it and nothing more is recorded, unless the hook reported that edit as failed; otherwise it is recorded as an edit with no lines whose source is the watcher, for the session at work at the time: of the sessions not ended before the edit, the one with the latest event from 30 s before it to 2 s after it (of two as late, the one whose id sorts first), a subagent's events counting for the session that started it; in that session's open task, which keeps the edit's content from before; or for no session. It is weighed only against events' times, so the order they arrived in does not change it |
| `judge`, `reverted_after`, `Judgement`, `FileNow`, `TaskEnd` | What became of a task that has ended, from its state and what each file it edited holds at present, with the words that say why for `tasks.outcome_evidence`. The rules in the order they are tried: a state that shows only part of the task says nothing; a task is reverted when it edited something and every file it edited holds again what it held before it (the same contents, or no file either time) or a git restore after that file's last edit put it back (everything, the file, or a folder above it); otherwise the last check that finished after the last edit tells, a test, a build or a type check, which exited 0 for a success and anything else for a failure, while a lint, a command that was interrupted or left in the background, and a check a git restore of an edited file followed all vouch for nothing; otherwise a task left before the end of an answer (the next prompt, or the session's end, with no end of the answer) was abandoned; nothing else is known. `reverted_after` is the revert the task after a task shows, by what the files hold when it ends or by a git restore in either task after the edit it undoes. What a file holds must have been read after the task last edited it (`FileNow` carries when it was read): until the index reads a change, the store still holds what the file held before it, which is the very content a revert is judged against. Both are pure, so a task always gets the same judgement in the same words |
| `interest_of`, `add_interest`, `EarnedInterest` | What a task's work adds to each node it touched, and the tier each score puts a node in. An edit earns 1.0, a test or build that passed after an edit 0.5 to the file it edited, a read of chosen lines 0.4, a read of a whole file 0.3, a file a search found and the task then opened 0.2, one it never opened 0.05. An edit or a read of chosen lines earns the symbols those lines touch, each of them also earning its file half as much and the folder holding the file a quarter; lines that cover no symbol, a read with no lines and a file with no symbols earn the file itself. A check earns each file edited before it once, however many times it was edited, and only a test or a build, exited 0. `add_interest` adds what was earned to what the decay of the tasks since leaves of each stored score, then chooses the landmarks again from what the store holds: the highest scoring files still at or above 3.0, one file in a hundred of the repository's own rounded up and held between 5 and 50, ties going to the smaller node. A score above 0.2 is interesting, at or below it fog |
| `links_of`, `add_links`, `EarnedLinks`, `Linking` | The links a task's work draws between the nodes it used together, as undirected edges of the graph. A node it edited counts 1.0, one it read that a node it edited names 0.6, one it only read 0.2; the set is cut to 40 nodes, the highest by role, then by interest, then the smaller node. Every pair of it is linked, between files and between symbols but never across, and gains the learning rate times the two roles times what the outcome teaches: a success in full, a task nobody can vouch for three tenths, a failed or left task nothing. A reverted task instead weakens the links between the files it edited together by 0.05 a pair. `add_links` adds each to what the decay of the tasks since leaves of the stored weight, holds the result between 0 and 5, counts one more observation behind it, and leaves a pair with no link and nothing to gain unwritten |
| `Commands::new`, `Commands::recognise`, `Ran`, `Check`, `Restore`, `CONFIGURED`, `PatternError` | What a shell command line ran, from the folder it ran in, in the order of the line, as far as its exit status vouches: each command told ended its pipeline, sits in the line's last and-or list, ran whenever what came before it there succeeded, and is followed by `&&` alone, neither inverted by `!` nor run in the background, so that when the line exits 0 each of them ran and exited 0. A check is a runner (npm, pnpm, yarn and bun scripts and their test commands, vitest, jest, pytest, unittest, go, cargo, Maven, Gradle, rspec, phpunit, dotnet, make, tsc, eslint, ruff, mypy) and what it checks, a `cortex_core::CommandKind` (tests, a build, types, lint), the variable assignments before it and the programs that only run another (`env`, `timeout`, `npx`, `uv run`, `python -m` and the like) passed over; or a command one of `tests.extra_patterns` names, as tests. A git restore names what it put back: `git restore` (not of the index alone), `git checkout -- <paths>` with or without a commit, everything for `git reset --hard`, its paths resolved from the folder (moved by `cd` and `git -C`) without reading the disk. The line is read as the POSIX shell reads it; one with a compound command (`if`, `for`, `case`), a function, an unclosed quote, or groups nested more than 16 deep tells nothing |

## Tests

```sh
cargo test -p cortex-learn
```

The tests record event sequences in the orders Claude Code sends them (a whole task, `/compact`,
`/clear`, a subagent, a lost Stop, an unknown session) into a real store and compare every row
with rows written by hand. The watcher's edits are tested the same way: how long each is held,
when a hook's edit claims it (a failed one does not), and which session gets it at the edges of the
30 s window, after a session ends, for a subagent and on a tie. One test searches the database's
files for marker words placed in a prompt, a command and a search pattern. The command lines of
every runner, what the exit status vouches for, git restores and their paths are tables of real
lines with the answers written by hand; two properties check that any text reads twice the same
without a panic, and that what follows a line built from known commands decides what it vouches
for. The task states are compared with steps written by hand: a task's reads, edits, searches and
shell commands with failed tools left out, files indexed after their event given their nodes at
the end, a state rebuilt after a restart, after a rolled-back batch and from a row that cannot be
read back, the cut at 4,096 rows for a task ended by its answer's end and by the next prompt, the
found files' bound, the state touched longest ago dropped for room, and what each edited file
held before the task: a hook's edit before the index saw the change and after it, an edit only
the watcher saw, a failed edit, a new file, and the first content kept. What became of a task is a
table of tasks recorded one by one with the outcome and the words written out by hand for each: a
check that passed and one that failed after the last edit, a check before it and one between two
edits, a lint, a check that never finished, nothing run and nothing edited, every edited file back
and one of them left changed, contents the index does not read, a file created and gone again and
one deleted, a git restore of a file, of a folder, of everything, one before the edit, one that
failed and one that covered only part of the task, a revert part by contents and part by a
restore, a content read before the edit and after it, a check a restore followed and one followed
by a restore of a file the task never edited, a revert outranking a passing check, a task left by
the next prompt and by its session's end, a state that shows only part of the task, and a revert
the next task shows, including one its steps do not show. The interest a task earns is one worked
example of every increment at once, with the symbols of a file given lines of their own and the
sums written out by hand, and then: work that vouches for nothing (a failed check, a lint, a read
of a file the store does not hold), what decay leaves of a score two tasks later, the landmarks a
repository of seven files allows, ties going to the smaller node, a file pushed out of the
landmarks, and a landmark that fades into the fog as the tasks go by. How many landmarks a
repository's size allows and where the fog ends are tables of their own, as is what decay leaves of
a weight, with a property that decay only ever takes from one. The links a task draws are worked
examples too: a successful task's six pairs across two levels with the three roles in the numbers,
what each outcome teaches, a reverted task weakening only the files it edited together, a weight
added to what decay leaves and held at its ceiling and at nothing, a pair with nothing to lose left
unwritten, and a task of 45 files keeping the 40 that mattered most; a property draws and weakens
one link over many tasks to watch its weight stay within bounds.
