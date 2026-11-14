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
when the work order is written. When the evidence does not agree, it adds nothing. After an
edit it says what the edit seems to leave undone: calls to a changed signature, the file's own
tests, a file that usually changes with it. Before a read it says when those lines were read
already, and after the agent's context is compacted it recalls what the task was doing. Codex
and the other agents are not supported yet. This page lists a feature only once it works and has tests,
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

`cortex init` sets Cortex up in the repository you run it in, for the coding agents it finds on
this machine (`--agents` names them instead); with none found it writes no hooks and says so.
Claude Code needs to be 2.1.295 or later. It asks two questions once: whether to measure
Cortex's effect by leaving one task in ten without its help (yes by default), and whether to share
facts with your team through a committed `.cortex/shared/` folder (no); `--yes` takes those
defaults. It registers Cortex's hooks with Claude Code in
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
transcripts, the tokens Cortex added, how well the session used the prompt cache (and where it
lost it), and the share of its work orders the task then used.
`cortex uninstall` puts back every file `init` changed and stops the daemon; `--purge` also deletes
the repository's `.cortex/` folder.

One daemon serves a repository, from its `.cortex` folder. It listens on a socket (a named pipe on
Windows) only its user can open, and stops after a while with nothing to do; a hook starts it again
when Claude Code next needs it. On Windows the daemon is started in two steps so that it holds none
of the handles of whatever started it (`cortex init`, `cortex status` or a hook, or the agent that
ran the hook): a script or an agent reading that command's output through a pipe sees it end with
the command, not with the daemon.

## What has been measured

Evaluations with Claude Code, in `eval/`: tasks on three pinned open-source projects (31 on a
Python toolkit of about 850 source files, 6 on a Python library of 15 files, 3 on a TypeScript web
framework of 190 source files), each run in a fresh copy with Cortex installed. Some tasks first
run warm-up sessions in the same area, as earlier work would; only the last session counts. Every
task runs the same way twice: once with Cortex giving work orders, once with Cortex installed but
holding every task out, and each task's runs are paired (`cortex-eval compare`, 95% intervals from
resampling tasks).

The latest, on 2026-11-14 (`eval/results/2026-11-14-8623dca-treated/` and `-held-out/`): 40 tasks,
3 runs each in each arm, 240 runs, Claude Code 2.1.318 with Sonnet 5.5 at medium effort.

- The work orders are accurate. The measured sessions got 90; 68 were used, meaning the session
  edited one of the work order's first three files, or read one and edited a file that names it: a
  precision of 0.76 (95% interval 0.60 to 0.89). Every one of the 6,141 code lines shown was the
  line its file held.
- No saving is measured. With Cortex the measured sessions took 8,593 more input tokens on average
  (interval 15,425 fewer to 32,131 more), 0.04 fewer turns (0.70 fewer to 0.66 more), and each run
  cost $0.002 less ($0.008 less to $0.004 more). Every interval spans zero. One task's three runs
  differ from each other by more than that: a median standard deviation of 27,076 input tokens
  with Cortex and 22,135 without.
- With Cortex every run's check passed (120 of 120); held out, 118 of 120 did (difference 0.02,
  interval 0.00 to 0.04).
- Everything Cortex said to the agents took at most 2.41% of the sessions' input tokens.
- After edits Cortex gave 98 warnings; the file 35 of them named was read or edited later in the
  task. Most edits in these tasks did not need the file a warning named.

So, as measured, Cortex gives an agent correct context it uses, and does not yet make the agent's
work cheaper.

## Build and test

```sh
cargo install --locked cargo-deny
cargo xtask ui
cargo xtask verify
```

`cargo xtask ui` installs the map's packages with npm and builds the map into `ui/dist`, which the
daemon's build embeds; run it before a release build, or the binary serves a page saying it has no
map. `cargo xtask verify` runs every check a change must pass: formatting, the rules for how crates
may depend on each other, the shared lint settings in every crate, the tags that mark where each
product guarantee is enforced, clippy, the tests, the dependency licence and advisory checks, and,
once `cargo xtask ui` has installed the map's packages, the map's type check, linters, unit tests,
build and bundle size (at most 1.5 MB gzipped); without them it says that it skipped the map.
`cargo xtask verify --full` also runs the map's browser checks and the benchmarks, and fails when
one is over its budget.
[CONTRIBUTING.md](CONTRIBUTING.md) explains the layout and how to send a change.
