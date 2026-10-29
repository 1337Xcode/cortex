# Contributing to Cortex

## Set up

1. Install [rustup](https://rustup.rs). The first `cargo` command in this repository installs the
   pinned toolchain from `rust-toolchain.toml`, with `rustfmt` and `clippy`.
2. Install cargo-deny, which checks dependency licences and advisories:

   ```sh
   cargo install --locked cargo-deny
   ```

3. Run every check:

   ```sh
   cargo xtask verify
   ```

   A green run ends with `xtask: all checks passed`. A change is ready when this passes.

## Layout

| Path | Holds |
| --- | --- |
| `crates/cortex-cli` | The `cortex` binary: commands and the entry points agents call |
| `crates/cortex-core` | Shared types: repository paths, ids, hashes, languages and stored names |
| `xtask` | The repository checks behind `cargo xtask verify` |
| `tests` | Cross-crate tests and the recorded agent hook payloads they read, in `fixtures/<agent>/<version>/` |

Further crates join as their code lands. Each crate has a place in a layered order, listed in
`xtask/src/architecture.rs`, and may depend only on crates below it; `cargo xtask verify` refuses a
dependency that points the other way.

## Adding a command

A command is one module in `crates/cortex-cli/src/commands/`, one variant of the `Command` enum and
one match arm in `crates/cortex-cli/src/commands/mod.rs`. Add a test in `crates/cortex-cli/tests/`
that runs the built binary.

## Code and tests

- Every module, type and function has a doc comment. Comments inside a function explain what a
  reader would stop at, not what the next line does.
- Tests assert values you worked out yourself from the input, never values copied from the
  program's own output. Cover the failure paths as well as the happy one.
- Never `unwrap` or `expect` outside tests. The workspace lints in `Cargo.toml` turn these and other
  slips into errors.

## Commits

One logical change per commit, with its tests, passing `cargo xtask verify` on its own. The message
is one line:

```text
type(scope): imperative summary
```

- `type` is one of `feat`, `fix`, `perf`, `refactor`, `test`, `docs`, `build`, `ci`, `chore`,
  `revert`.
- `scope` is a crate without its `cortex-` prefix (`cli`), or one of `xtask`, `tests`, `docs`,
  `deps`, `repo`.
- At most 72 characters, lower case after the colon, no full stop.

Example: `feat(cli): print the version with cortex version`.

## Security issues

Do not open a public issue for a vulnerability. See [SECURITY.md](SECURITY.md).
