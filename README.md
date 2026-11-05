# Cortex

Cortex is a local memory and map of a codebase for coding agents. It is being built to run beside
Claude Code and Codex through their official hooks, to watch what agents read, edit and test, to
learn which code belongs together from structure, git history and finished work, and to hand the
next task the exact code and project facts it needs. Two rules shape the design: stay silent when
unsure, and keep everything on the user's machine.

## Status

Cortex is in early development. The `cortex` binary sets a repository up for Claude Code, indexes
it and keeps the index current as files change. The hooks it installs hand Claude Code's events to
Cortex, but nothing goes back to the agent yet: that comes next. This page lists a feature only once
it works and has tests, and a number only once it has been measured.

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
is ready, the repository's scale mode and how many files are stored and parsed. `cortex doctor`
checks that Claude Code will run Cortex's hooks for the repository and that the daemon answers them
in time, and prints the fix for each failure.

One daemon serves a repository, from its `.cortex` folder. It listens on a socket (a named pipe on
Windows) only its user can open, and stops after a while with nothing to do; a hook starts it again
when Claude Code next needs it. On Windows the daemon is started in two steps so that it holds none
of the handles of whatever started it (`cortex init`, `cortex status` or a hook, or the agent that
ran the hook): a script or an agent reading that command's output through a pipe sees it end with
the command, not with the daemon.

## What has been measured

Nothing yet. Results will appear here only from the evaluation in this repository, with how they
were measured and how many runs they rest on.

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

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.
