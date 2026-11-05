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
| `crates/cortex-cli` | The `cortex` binary: every user command and the entry points agents call |
| `crates/cortex-core` | Shared types: repository paths, ids, hashes, languages, stored names, record kinds and settings; finding and running another program, bounded in time and output |
| `crates/cortex-daemon` | The daemon: one process per repository that keeps its index current, records what agents do, holds the graph and answers retrievals; the only writer of the database |
| `crates/cortex-git` | Reading a repository's git history and work tree: the commits on the first-parent line, the files each one changed, and the uncommitted files |
| `crates/cortex-hook` | What runs when an agent calls `cortex hook`: the agent adapters, the fail-open client of the daemon, and starting the daemon when none runs |
| `crates/cortex-index` | The indexer: discovering a repository's files, flagging secrets, binaries, generated and vendored code, and reading what each file defines, uses and imports |
| `crates/cortex-learn` | Learning from agents' work: sessions, tasks and events, what became of each task, interest, links between code used together, and the labels of work orders |
| `crates/cortex-meter` | What agents' work costs in tokens, read from their transcripts, and the comparison of held-out tasks with the rest |
| `crates/cortex-retrieve` | Retrieval: seeds from a prompt and a session, spreading activation over the graph, the gate, and the work order and primer it renders |
| `crates/cortex-store` | The SQLite database in `.cortex/`: migrations, the single writer and readers |
| `eval` | The evaluation suite: tasks, testbeds, checks and the runner that has real agents work through them |
| `fuzz` | Fuzz targets for the parsers that read input Cortex does not control: hook input and the daemon's frames |
| `xtask` | The repository checks behind `cargo xtask verify` |
| `tests` | Cross-crate tests and the recorded agent hook payloads they read, in `fixtures/<agent>/<version>/` |

Each crate has a place in a layered order, listed in `xtask/src/architecture.rs`, and may depend
only on crates below it; `cargo xtask verify` refuses a dependency that points the other way.

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

## Adding an agent

An agent reaches Cortex through its hooks. On the hook side, an agent is one adapter: a module in
`crates/cortex-hook/src/` (Claude Code's is `claude/`) implementing the `Adapter` trait in
`crates/cortex-hook/src/adapter.rs` (the hooks Cortex registers, the agent versions it knows, how a
payload becomes a normalized event, and how text is printed for the agent), and one line in that
file's `ADAPTERS` list. Its tests read real payloads recorded from the agent, scrubbed, under
`tests/fixtures/<agent>/<version>/`.

Writing the hooks into the agent's own settings is `cortex init`'s job, which today knows Claude
Code only (`crates/cortex-cli/src/commands/init/hooks.rs`).

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
- `scope` is a crate without its `cortex-` prefix (`cli`), or one of `ui`, `eval`, `tests`,
  `fuzz`, `bench`, `xtask`, `ci`, `docs`, `release`, `deps`, `repo`.
- At most 72 characters, lower case after the colon, no full stop.

Example: `feat(cli): print the version with cortex version`.

## Sending a pull request

Fill in the template's sections. **Verification** carries the proof: the last lines of
`cargo xtask verify` from your machine and what you ran by hand, such as a `cortex` command on a
repository of your own, with what it printed. One problem per pull request; for a new feature,
start a discussion first so the idea is agreed before you spend time on it.

A workflow reads each new pull request and may add labels:

| Label | When |
| --- | --- |
| `new-account` | The account was less than 30 days old when the pull request was opened, and it is the author's first here |
| `fresh-fork` | The pull request came within 24 hours of its fork, from an author with nothing merged here yet |
| `needs-proof` | The Verification section is empty |

It also leaves one comment saying what a maintainer will look for, and edits that comment when
the labels change. It never closes a pull request, never requests changes and never runs the pull
request's code. A maintainer can clear the labels at any time with `maintainer-ok`. Once you have
work merged, a maintainer can add your login to `.github/VOUCHED`; the workflow reads that file
from `main` and leaves your pull requests unlabelled.

## License

Cortex is licensed under the Apache License, Version 2.0. By sending a change you agree to
license it under the same terms.

## Security issues

Do not open a public issue for a vulnerability. See [SECURITY.md](SECURITY.md).
