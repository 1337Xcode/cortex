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
   (`src/invariants.rs`). A tag is a `//` line comment outside string literals. The scan never
   follows symbolic links and leaves out `.git`, `node_modules`, `ui/dist`, a package's own
   `target` folder and `tests/fixtures`.
5. `cargo clippy --workspace --all-targets --locked -- -D warnings`
6. `cargo test --workspace --locked`, doc tests included
7. `cargo deny check`: licences, advisories, bans and sources (`deny.toml`)

## Tests

```sh
cargo test -p xtask
```

The architecture, lint and tag checks are pure functions tested on synthetic input; the file
walk is tested on a temporary folder, and the manifest checks on this workspace's own
`cargo metadata`.
