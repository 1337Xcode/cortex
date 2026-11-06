# Changelog

All notable changes to this project are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `cortex init` sets a repository up for Claude Code: it finds the repository with git, checks
  Claude Code's version, asks three questions once (the holdout, team sharing, model upkeep),
  registers Cortex's hooks in `.claude/settings.local.json` (or `~/.claude/settings.json` with
  `--scope user`) beside the user's own, backs up every file it changes with a hashed manifest,
  starts the repository's daemon, waits for its first index and ends with `cortex doctor`.
- `cortex uninstall` restores every file init changed from its manifest and stops the daemon;
  `--purge` also deletes the repository's `.cortex/` folder, except `.cortex/shared/`, and its
  manifests and daemon log in the user's folder.
- `cortex doctor` checks that Claude Code will run Cortex's hooks for the repository and that the
  daemon answers each hook within its latency budget, and prints the fix for each failure.
- `cortex status` reports the daemon (process, version, uptime, queue, hook latency percentiles,
  recent errors), the index and scale mode, the git history mined, the work tree read, the graph,
  the gate's bar and what the self-check last found.
- One daemon per repository keeps an index current as files change. Files are discovered with
  secret, binary and generated files left out; symbols, references and imports are parsed for
  Python, JavaScript, TypeScript and TSX, Go, Rust, Java, Ruby, C, C++, C#, PHP, Kotlin, Swift,
  Scala and Bash, plus Markdown headings, SQL definitions and top-level JSON, YAML and TOML keys;
  imports are resolved to files, and a SCIP index can supply references at full confidence. Large
  repositories are indexed in a lighter mode. On Windows the daemon holds none of the handles of
  what started it, so an agent or script reading a hook's or command's output through a pipe sees
  it end with that process.
- Through Claude Code's hooks, the daemon records what agents read, edit and run, as sessions,
  tasks and events, never the content of files, prompts or tool output.
- Co-change mining reads the first-parent git history (with `gix`, or the `git` program when `gix`
  cannot) into weighted links between files that change together.
- Learning: when a task ends, Cortex judges what became of it, earns interest for the code it
  used, draws links between code used together and groups similar tasks.
- Retrieval: from a prompt, Cortex seeds the graph from the words and paths it names, the
  session's own work, the uncommitted files and similar past tasks, spreads relevance over files
  first and then over the best files' symbols. It gives a work order (the files and line ranges
  to start from, each with its reason, checked against the file on disk when it is written) only
  when the evidence agrees, and nothing otherwise. Each session starts with
  a short, byte-stable primer.
- The gate's self-check raises the bar when the work orders given go unused.
- `cortex where "<question>"` prints what the engine finds for a question, or why it finds
  nothing.
- `cortex receipts` shows each session's and task's token usage and estimated cost, read from
  Claude Code's transcripts, the tokens Cortex injected, the share of its work orders that were
  used, and, once each arm has at least 30 ended tasks, the difference between the tasks Cortex
  held out and the rest, each difference with a 95% bootstrap interval.
- An evaluation suite (`eval/`): tasks that Claude Code works through in fresh copies of a pinned
  testbed, with success checks and a report of precision and usage, each share with a bootstrap
  interval that resamples whole tasks; `cortex-eval report` rebuilds a results folder's summary
  from its runs. The first measured results are in `eval/results/2026-11-05-c856abc/`.
- `cortex version` prints the binary's name and version.
- After Claude Code edits a file, it is told what the edit seems to leave undone: when a
  function's signature changed, the calls to it in other files the task has not edited (how many,
  and five listed by path and line, each line checked against the file on disk); the file's tests,
  once a task; and a file that usually changes with it in the repository's history or in earlier
  tasks, when the task has not touched it. At most three warnings an edit, strongest first, each
  file named at most once a task, within 250 tokens; `warnings.max_per_edit`,
  `warnings.cochange_min_confidence` and `warnings.cochange_min_count` set the limits.
- Before Claude Code reads a file, it is told when the session already read those lines since its
  context was last compacted and the file has not changed since, or, in a file over 600 lines,
  which parts of it the task's work order pointed at, each line checked against the file on disk,
  within 120 tokens. A tiny repository gets no hint.
- When a task ends, each warning its edits got is marked resolved if the agent then read or edited
  the file it named. `cortex status` and `cortex receipts` show the share of warnings followed up
  this way.
- After Claude Code compacts a session's context, the start that follows is told what the task in
  progress was doing: the files it edited, the files it was warned of and has not read or edited
  since, and the start of its prompt when `privacy.store_prompt_excerpt` kept it, within 500
  tokens, once for each compaction.
- `strict_mode = true` gives Claude Code a reason to go on when it stops while files calling a
  signature its task changed were not read or edited since the warning, once a task. Off by
  default, when a stop is given nothing. The hook and the daemon now speak protocol 5.0: a hook and
  a daemon of different Cortex versions are replaced as before.
- A work order no longer shows code the session's agent was shown by an earlier work order, or
  read, since its context was last compacted, while the file is unchanged.
- A slash command, a shell escape (`!`) or a prompt of fewer than four characters such as "ok" asks
  for no work order: the hook sends the prompt to be recorded, and the daemon retrieves nothing.
- Every text Cortex gives a task is recorded with its tokens and counted in `cortex receipts`: work
  orders as before, and now an edit's warnings, a read's hint, the recap after a compaction and
  strict mode's reason. A held-out task is given none of them, so the comparison between held-out
  tasks and the rest holds out everything Cortex would say to a task.
- `cargo xtask verify` runs formatting, the crate dependency-direction rules, the shared lint
  settings in every crate, the invariant tags, clippy, the tests and the dependency licence and
  advisory checks; `--full` also runs the benchmarks against their budgets.

### Changed

- A file the prompt names, by its path or by a rare name defined in it, now leads the work order
  with its code, and the folders of a path the prompt names are no longer search words: a prompt
  naming `src/app/helpers.py` no longer ranks every file under `src/app` above the one it names.
- Cortex is licensed under the Apache License, Version 2.0, alone (see `LICENSE` and `NOTICE`).
  Commits before this change were offered under the MIT license or Apache-2.0, at the user's
  option, and stay available under those terms.

### Fixed

- The lines `cortex init` adds to a `.gitignore` no longer count as uncommitted work: a
  prompt right after install is not taken to continue a change of the `.gitignore`.
- A prompt's retrieval no longer walks every symbol node of the repository: the store's lookups of
  a named symbol, of a listed file's symbols and of the files of nodes start from their own index,
  and the co-change pairs of a file are found by either of its sides through an index (a schema
  migration adds it). On a 10,000-file test repository a retrieval in the daemon went from 136.9 ms
  to 33.3 ms at the 95th percentile on Windows.
- While a large index runs on a busy machine, hook events reach the store within a second again:
  the index writes references and edges in transactions cut every quarter second rather than every
  4,096 rows, reads the whole repository's names in steps, and walks the repository on a thread of
  its own while it records the events waiting.
