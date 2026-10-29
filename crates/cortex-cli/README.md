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

Adding a command means one new module in `src/commands/`, one variant of `Command` and one match
arm in `src/commands/mod.rs`.

## Tests

```sh
cargo test -p cortex-cli
```

The tests in `tests/` run the built binary.
