# cortex-learn

What Cortex learns from agents' work. For now, the record of it: each normalized hook event becomes
the session, task and event rows the store keeps.

## What it must never do

- Keep a prompt's text (a task keeps the prompt's hash, and its first 200 characters only when
  `privacy.store_prompt_excerpt` is on), a command, a search pattern or anything a tool printed.
- Write the store itself: it records inside a transaction the daemon's single writer opens.

## Entry points

| Item | What it is |
| --- | --- |
| `Recorder::new`, `Recorder::record` | Records one event in a write transaction. A prompt starts a task numbered across the repository, ending first any task its session left open when the end of the previous answer never arrived; the end of the answer or of the session ends it as `unknown`. A subagent is a session of its own with its parent's id, and its events belong to the parent's open task. A tool call is recorded after it ran, as a read, a search, an edit (one row per changed stretch of lines, up to `cortex_core::MAX_RANGES`), a shell command or another tool, with the file's graph node when the file is indexed; the moment before a tool runs is not kept. An event from a session the store has not seen registers the session |

## Tests

```sh
cargo test -p cortex-learn
```

The tests record event sequences in the orders Claude Code sends them (a whole task, `/compact`,
`/clear`, a subagent, a lost Stop, an unknown session) into a real store and compare every row
with rows written by hand. One test searches the database's files for marker words placed in a
prompt, a command and a search pattern.
