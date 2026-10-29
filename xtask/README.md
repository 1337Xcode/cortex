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
3. Invariant tags: every tag comment names a known invariant, and each invariant whose owning
   crate exists has at least one tagged enforcing line inside that crate (`src/invariants.rs`).
4. `cargo clippy --workspace --all-targets --locked -- -D warnings`
5. `cargo test --workspace --locked`, doc tests included
6. `cargo deny check`: licences, advisories, bans and sources (`deny.toml`)

## Tests

```sh
cargo test -p xtask
```

The architecture and tag checks are pure functions tested on synthetic input.
