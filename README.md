# Cortex

Cortex is a local memory and map of a codebase for coding agents. It runs beside the agent through
the agent's official hooks, watches what the agent reads, edits and runs, learns which code belongs
together, and hands the next task the code it needs before the agent goes looking. Two rules shape
it: stay silent when unsure, and keep everything on your machine.

## Status

Cortex is in early development and works with Claude Code. The `cortex` binary sets a repository
up, indexes it and keeps the index current as files change, records what Claude Code reads, edits
and runs, mines the git history for files that change together, and learns from each finished
task. When a prompt points at code the evidence agrees on, Cortex gives Claude Code a short work
order: the files and line ranges to start from, each with its reason, read from the files on disk
when the work order is written. When the evidence does not agree, it adds nothing. Codex and the
other agents are not supported yet. This page lists a feature only once it works and has tests,
and a number only once it has been measured.

## Install

There is no release yet. To build from source you need Rust (rustup reads the pinned toolchain from
`rust-toolchain.toml`):

```sh
cargo install --path crates/cortex-cli
```

## Use

```sh
cd path/to/repository
cortex init
cortex status
cortex doctor
cortex where "where is the tax rate applied"
cortex receipts
cortex uninstall
```

`cortex init` sets Cortex up in the repository you run it in. It needs Claude Code 2.1.295 or
later. It asks three questions once: whether to measure Cortex's effect by leaving one task in ten
without its help (yes by default), whether to share facts with your team through a committed
`.cortex/shared/` folder (no), and whether to allow optional upkeep by a language model (no);
`--yes` takes those defaults. It registers Cortex's hooks with Claude Code in
`.claude/settings.local.json` (or, with `--scope user`, in `~/.claude/settings.json`), adds what
Cortex writes to `.gitignore`, backs up every file before it changes it, starts the repository's
daemon, waits for its first index and ends with `cortex doctor`'s checks, whose failures it
reports without failing. Running it again writes nothing new.

`cortex status` asks the daemon whether it runs (its process, version and uptime, what waits to be
recorded, how long it takes to handle each hook, and its latest errors), and tells whether the index
is ready, the repository's scale mode and how many files are stored and parsed, how much git
history was read, and how the gate is set. `cortex doctor` checks that Claude Code will run
Cortex's hooks for the repository and that the daemon answers them in time, and prints the fix for
each failure.

`cortex where` prints what Cortex would give an agent for a question: the work order, or why there
is none. `cortex receipts` shows each session's token usage and estimated cost from Claude Code's
transcripts, the tokens Cortex added, and the share of its work orders the task then used.
`cortex uninstall` puts back every file `init` changed and stops the daemon; `--purge` also deletes
the repository's `.cortex/` folder.

One daemon serves a repository, from its `.cortex` folder. It listens on a socket (a named pipe on
Windows) only its user can open, and stops after a while with nothing to do; a hook starts it again
when Claude Code next needs it. On Windows the daemon is started in two steps so that it holds none
of the handles of whatever started it (`cortex init`, `cortex status` or a hook, or the agent that
ran the hook): a script or an agent reading that command's output through a pipe sees it end with
the command, not with the daemon.

## What has been measured

One evaluation so far, with Claude Code, in `eval/`. Twelve tasks on a pinned copy of an
open-source Python project of about 850 source files, each run four times in a fresh copy with
Cortex installed. In each run, one or two warm-up sessions work in the same area first, as earlier
work would, and then a new session makes the change; only the new session's work orders count. The
copies had no git history, so Cortex worked from the code's structure, the prompt and what the
warm-up sessions did.

- Work orders reached the measured session in 32 of 48 runs: every run of 8 tasks. In the 16 runs
  of the other 4 tasks the evidence did not agree and Cortex added nothing.
- 29 of the 32 were used, meaning the session edited one of the work order's first three files, or
  read one and edited a file that names it. That is a precision of 0.91 (95% interval 0.70 to 1.00,
  resampling tasks). All 3 unused work orders were in one task.
- The tasks' checks passed in 47 of 48 runs.

This measures how often Cortex points at the right code when it speaks. It does not yet measure
whether that saves tokens, turns or time: that needs the same tasks run with Cortex holding them
out, which has not been done. Each run's counts are in `eval/results/2026-11-05-c856abc/`.

## Build and test

```sh
cargo install --locked cargo-deny
cargo xtask verify
```

`cargo xtask verify` runs every check a change must pass: formatting, the rules for how crates may
depend on each other, the shared lint settings in every crate, the tags that mark where each
product guarantee is enforced, clippy, the tests and the dependency licence and advisory checks.
`cargo xtask verify --full` also runs the benchmarks and fails when one is over its budget.
[CONTRIBUTING.md](CONTRIBUTING.md) explains the layout and how to send a change.
