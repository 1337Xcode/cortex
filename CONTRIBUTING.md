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
| `crates/cortex-core` | Shared types: repository paths, ids, hashes, languages, stored names, record kinds and settings |
| `crates/cortex-index` | The indexer: discovering a repository's files, flagging secrets, binaries, generated and vendored code, and reading what each file defines, uses and imports |
| `crates/cortex-store` | The SQLite database in `.cortex/`: migrations, the single writer and readers |
| `xtask` | The repository checks behind `cargo xtask verify` |
| `tests` | Cross-crate tests and the recorded agent hook payloads they read, in `fixtures/<agent>/<version>/` |

Further crates join as their code lands. Each crate has a place in a layered order, listed in
`xtask/src/architecture.rs`, and may depend only on crates below it; `cargo xtask verify` refuses a
dependency that points the other way.

## Adding a command

A command is one module in `crates/cortex-cli/src/commands/`, one variant of the `Command` enum and
one match arm in `crates/cortex-cli/src/commands/mod.rs`. Add a test in `crates/cortex-cli/tests/`
that runs the built binary.

## Adding a language

A language is one file in `crates/cortex-index/src/languages/` and one line in the `ALL` list in
`crates/cortex-index/src/languages/mod.rs`. The file says how a file is recognised: extensions,
whole file names, `#!` interpreters and `linguist-language` names. For a language Cortex parses,
it also gives the tree-sitter grammar, the tags query (the grammar's own and additions in
`crates/cortex-index/queries/<language>-tags.scm`, in the order they should win, since the
earlier of two patterns that find the same name wins), the imports query
(`crates/cortex-index/queries/<language>-imports.scm`), the grammar fields that hold a
definition's body, and how the language names test files and test functions. Add a sample file
and the output you expect from it, written by hand, under
`crates/cortex-index/tests/languages/<language>/`, and a test for it in
`crates/cortex-index/tests/extract.rs`.

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
