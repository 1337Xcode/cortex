# Cortex

Cortex is a local memory and map of a codebase for coding agents. It is being built to run beside
Claude Code and Codex through their official hooks, to watch what agents read, edit and test, to
learn which code belongs together from structure, git history and finished work, and to hand the
next task the exact code and project facts it needs. Two rules shape the design: stay silent when
unsure, and keep everything on the user's machine.

## Status

Cortex is in early development. The `cortex` binary indexes a repository and keeps the index
current as files change, but nothing reads the index yet: the hooks that bring it to agents come
next. This page lists a feature only once it works and has tests, and a number only once it has
been measured.

## Install

There is no release yet. To build from source you need Rust (rustup reads the pinned toolchain from
`rust-toolchain.toml`):

```sh
cargo install --path crates/cortex-cli
```

## Use

```sh
cortex daemon --root path/to/repository
cortex status --root path/to/repository
cortex version
```

`cortex daemon` indexes the repository (the current folder when `--root` is left out) into its
`.cortex` folder, then watches it and indexes each change, and stops after a while with nothing to
do. One daemon serves a repository; it listens on a socket (a named pipe on Windows) only its user
can open. `cortex status` asks the daemon whether it runs (its process, version and uptime, and what waits
to be recorded), and
tells whether the index is ready, the repository's scale mode and how many files are stored and
parsed.

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
