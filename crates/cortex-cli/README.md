# cortex-cli

Builds the `cortex` binary: every command a user types and, as the other crates land, the internal
entry points agents and the daemon call.

## What it must never do

- Hold product logic. A command parses its arguments, calls the crate that owns the behaviour and
  prints the result.
- Print command output anywhere but through `src/output.rs`, or panic on a closed pipe. Help,
  `--version` and usage errors are printed by clap.

## Entry points

| Command | What it does |
| --- | --- |
| `cortex version`, `cortex --version` | Prints `cortex <version>` |
| `cortex daemon [--root <folder>]` | Keeps the repository's index, the current folder's when no root is given, as its only writer: indexes what changed since the last index, then watches the repository and indexes again after each change. In huge mode it parses the files it took inventory of, the changed ones first, `index.crawl_files_per_second` a second on one thread. Writes `cortex: daemon ready: ...` on stderr after the first index and stops after `daemon.idle_shutdown_minutes` without a change once nothing waits to be parsed. A second daemon on the same repository is refused |
| `cortex status [--root <folder>]` | Prints whether a daemon holds the repository's store, whether the first index has finished, the scale mode the last index recorded (and the one the configuration pins, if any), and how many files are stored, parsed and waiting to be parsed; or that the repository has no index. Only reads the store |

Adding a command means one new module in `src/commands/`, one variant of `Command` and one match
arm in `src/commands/mod.rs`.

## Tests

```sh
cargo test -p cortex-cli
```

The tests in `tests/` run the built binary. The daemon tests start it on temporary repositories
with an empty home folder, so the user's own configuration never applies.

Configuration comes from `~/.cortex/config.toml`, then the repository's `.cortex/config.toml`; a
file over 1 MiB is refused, and an unknown key prints a warning.
