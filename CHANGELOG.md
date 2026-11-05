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

- A prompt's retrieval no longer walks every symbol node of the repository: the store's lookups of
  a named symbol, of a listed file's symbols and of the files of nodes start from their own index,
  and the co-change pairs of a file are found by either of its sides through an index (a schema
  migration adds it). On a 10,000-file test repository a retrieval in the daemon went from 136.9 ms
  to 33.3 ms at the 95th percentile on Windows.
- While a large index runs on a busy machine, hook events reach the store within a second again:
  the index writes references and edges in transactions cut every quarter second rather than every
  4,096 rows, reads the whole repository's names in steps, and walks the repository on a thread of
  its own while it records the events waiting.
