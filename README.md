# Cortex

Cortex is a local memory and map of a codebase for coding agents. It is being built to run beside
Claude Code and Codex through their official hooks, to watch what agents read, edit and test, to
learn which code belongs together from structure, git history and finished work, and to hand the
next task the exact code and project facts it needs. Two rules shape the design: stay silent when
unsure, and keep everything on the user's machine.

## Status

Cortex is in early development and does nothing useful yet. Today the `cortex` binary has one
command, `cortex version`. This page lists a feature only once it works and has tests, and a number
only once it has been measured.

## Install

There is no release yet. To build from source you need Rust (rustup reads the pinned toolchain from
`rust-toolchain.toml`):

```sh
cargo install --path crates/cortex-cli
```

## Use

```sh
cortex version
```

## What has been measured

Nothing yet. Results will appear here only from the evaluation in this repository, with how they
were measured and how many runs they rest on.

## Build and test

```sh
cargo install --locked cargo-deny
cargo xtask verify
```

`cargo xtask verify` runs every check a change must pass: formatting, the rules for how crates may
depend on each other, clippy, the tests and the dependency licence and advisory checks.
[CONTRIBUTING.md](CONTRIBUTING.md) explains the layout and how to send a change.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT)
at your option.
