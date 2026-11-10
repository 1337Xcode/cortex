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

Evaluations with Claude Code, in `eval/`: tasks on pinned open-source Python projects, each run in
a fresh copy with Cortex installed. Some tasks first run warm-up sessions in the same area, as
earlier work would; only the last session's work orders count.

The latest, on 2026-11-06 (`eval/results/2026-11-06-5d28da8/`): 17 tasks, 2 runs each, on a
project of about 850 source files (14 tasks) and one of 15 files (3 tasks), cloned with their git
history.

- Every run's check passed.
- The measured sessions got 18 work orders; 15 were used, meaning the session edited one of the
  work order's first three files, or read one and edited a file that names it: a precision of 0.83
  (95% interval 0.57 to 1.00, resampling tasks).
- Every one of the 1,300 code lines the sessions were shown was the line its file held.
- Everything Cortex said to the agents took at most 1.71% of the sessions' input tokens; in the
  15-file project, which gets no work orders, at most 0.20%.
- Warnings after edits named 32 files, of which 4 were read or edited afterwards. Most edits in
  these tasks did not need the file a warning named.

Whether Cortex saves tokens, turns or money is measured by running the same tasks twice, once with
Cortex and once with Cortex holding every task out, and pairing each task's runs
(`cortex-eval compare`). A first pilot, 15 tasks a side with 2 runs each
(`eval/results/2026-11-06-e8bee2b-treated/comparison.txt`, 95% intervals from resampling tasks):

- In the 8 tasks where Cortex gave the agent a work order, the measured sessions took 1.1 fewer
  turns and tool calls (7.4 against 8.6; interval 0.4 to 1.9 fewer), and each run cost about 13%
  less ($0.157 against $0.181; interval $0.009 to $0.041 less). Their input was 10,770 tokens
  lower on average, which these few tasks do not settle (interval 27,500 lower to 6,850 higher).
- In the 7 tasks where it gave none, the sessions took 1.6 more turns and 70,977 more input tokens
  (intervals 0.4 to 2.6 more turns, 36,986 to 104,311 more tokens). Those 7 tasks were run again
  on a later build, 2 runs each in each arm (`eval/results/2026-11-08-a9e546d-silent-treated/`),
  and the cost did not come back: with Cortex the sessions took 19,707 fewer input tokens
  (interval 95,424 fewer to 34,881 more), 0.64 fewer turns (2.43 fewer to 0.86 more) and $0.011
  less a run (0.026 less to 0.001 more). The pilot's difference there was not a steady effect of
  Cortex.
- Over all 15 tasks the input was 27,378 tokens higher (interval 2,285 to 56,418), the turns and
  tool calls the same within their interval (0.17 more; 0.77 fewer to 1.13 more), and each run
  cost $0.008 less (interval $0.021 less to $0.005 more).

Fifteen tasks a side is too few to state a saving overall. The next comparison runs 30 or more
tasks a side once more tasks get a correct work order and the warnings are cut.

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
