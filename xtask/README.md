# xtask

The one entry point for every check in this repository:

```sh
cargo xtask verify          # what every change must pass
cargo xtask verify --full   # adds the benchmark budget checks
```

`verify` runs, in order, and stops at the first failure:

1. `cargo fmt --all -- --check`
2. Architecture: the dependency direction between workspace crates, read from `cargo metadata`.
   Each product crate has a layer (`LAYERS` in `src/architecture.rs`) and may depend only on lower
   layers; `cortex-hook` may depend on `cortex-core` alone.
3. Workspace lints: every member's `Cargo.toml` sets `[lints] workspace = true`, so the lints in
   the workspace manifest reach it (`src/lints.rs`).
4. Invariant tags: every tag comment names a known invariant, and each invariant whose owning
   crate exists has at least one tagged enforcing line in that crate's `src/`
   (`src/invariants.rs`). A tag is a `//` line comment outside string literals and block
   comments, found by a small lexer (`src/comments.rs`). A tag marks the line it ends, or, alone
   on its line, the next line of code; only a tag whose line the product compiles enforces
   anything. The check parses every library and binary target with `syn`, follows its `mod`
   declarations the way the compiler does (every file a `#[path]` or `cfg_attr` path can choose),
   and leaves out code under an attribute named `test` or `bench` or under a `cfg` that cannot
   hold without `test` (`src/compiled.rs`, `src/cfg.rs`). Inside a macro's tokens such an
   attribute covers the next braced block or item, and each `cfg_if!` branch is judged with the
   earlier conditions negated. Test targets, benches, examples and files no target reaches never
   count. The scan never follows symbolic links and leaves out `.git`,
   `node_modules`, `ui/dist`, a package's own `target` folder and `tests/fixtures`.
5. `cargo clippy --workspace --all-targets --locked -- -D warnings`
6. `cargo test --workspace --locked --no-fail-fast`, doc tests included: every test binary runs
   even after one fails, so a single run reports every failing test
7. `cargo deny check`: licences, advisories, bans and sources (`deny.toml`)

`verify --full` then runs the slow tests (the same test command with `-- --ignored`) and the
benchmarks (`cortex-index`'s `index`, `cortex-retrieve`'s `spread`, `cortex-cli`'s `hooks`), and
checks the 95th percentile of each benchmark's single runs against its budget (`src/budgets.rs`).
The first index of 10,000 files must take 60 s at most and an index after one change 30 ms,
everywhere. Spreading over generated graphs of 10,000, 100,000 and 1,000,000 nodes, two-stage
spreading over the largest, and a prompt's retrieval in the daemon on a repository of 10,000 files
must each take 120 ms at most, everywhere. Each hook, from its
process's start to its exit, must take 120 ms for SessionStart, 150 ms for UserPromptSubmit, 40 ms
for PreToolUse on a read, 80 ms for PostToolUse on an edit, 60 ms for Stop and 10 ms with no input,
on macOS and Linux, where hook latency is specified. The first five hold too when the daemon is
down and the hook starts one. On Windows starting a process alone takes
most of those 10 ms, so the hooks' percentiles are printed there and not checked. Every budget's
line says what was measured.

## Tests

```sh
cargo test -p xtask
```

The architecture, lint and tag checks, the comment lexer and the test-code rules are pure
functions tested on synthetic input; the file walk and the module walk are tested on temporary
folders, and the manifest checks on this workspace's own `cargo metadata`.
