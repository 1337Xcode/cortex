# cortex-learn

What Cortex learns from agents' work. For now, the record of it: each normalized hook event becomes
the session, task and event rows the store keeps, and each edit only the file watcher saw is given
to the session most likely to have made it, or to none.

## What it must never do

- Keep a prompt's text (a task keeps the prompt's hash, and its first 200 characters only when
  `privacy.store_prompt_excerpt` is on), a command, a search pattern or anything a tool printed.
- Write the store itself: it records inside a transaction the daemon's single writer opens.

## Entry points

| Item | What it is |
| --- | --- |
| `Recorder::new`, `Recorder::record` | Records one event in a write transaction. A prompt starts a task numbered across the repository, ending first any task its session left open when the end of the previous answer never arrived; the end of the answer or of the session ends it as `unknown`. A subagent is a session of its own with its parent's id, and its events belong to the parent's open task. A tool call is recorded after it ran, as a read, a search, an edit (one row per changed stretch of lines, up to `cortex_core::MAX_RANGES`), a shell command or another tool, with the file's graph node when the file is indexed; the moment before a tool runs is not kept. An event from a session the store has not seen registers the session |
| `WatcherEdits`, `HOLD_MS`, `Recorder::record_watcher_edit`, `WatcherEdit`, `ATTRIBUTION_WINDOW_MS` | Edits the file watcher saw, each held 2 s from when it was first seen (`hold`, `next_due_ms`, `take_due`). Then a hook's edit of the same file at most 2 s before or after it claims it and nothing more is recorded; otherwise it is recorded as an edit with no lines whose source is the watcher, for the live session whose latest event is the most recent and at most 30 s older than the edit (of two as recent, the one whose id sorts first), in that session's task (a subagent's: its parent's), or for no session. A session's end takes it out of the running |

## Tests

```sh
cargo test -p cortex-learn
```

The tests record event sequences in the orders Claude Code sends them (a whole task, `/compact`,
`/clear`, a subagent, a lost Stop, an unknown session) into a real store and compare every row
with rows written by hand. The watcher's edits are tested the same way: how long each is held,
when a hook's edit claims it, and which session gets it at the edges of the 30 s window, after a
session ends, for a subagent and on a tie. One test searches the database's files for marker words placed in a
prompt, a command and a search pattern.
