# Contributing to Cortex

Thanks for your interest in contributing to Cortex. This document covers how to build, test, and submit changes.

## Prerequisites

- Rust 1.75+ (install via [rustup](https://rustup.rs/))
- A C compiler (for tree-sitter grammar compilation)
- Git

## Building

```sh
git clone https://github.com/1337Xcode/cortex.git
cd cortex
cargo build --release
```

The build compiles all tree-sitter grammars from source via `build.rs`. No network downloads happen at build time. First build takes 3-5 minutes. Incremental builds take 20-40 seconds.

For development iteration, use `cargo check` for fast type checking without full compilation.

## Running tests

```sh
# Run all tests
cargo test

# Run unit tests only
cargo test --lib

# Run integration tests only
cargo test --test '*'

# Run tests for a specific module
cargo test store::
cargo test indexer::
cargo test mcp::
```

Integration tests use fixture repositories in `tests/fixtures/`. Some tests require the binary to be built first:

```sh
cargo build --release
cargo test --test integration
```

## Project structure

```
src/
├── main.rs          Entry point, CLI wiring
├── config.rs        Configuration loading
├── error.rs         Top-level error types
├── mcp/             MCP server (JSON-RPC 2.0 over stdio)
├── indexer/         tree-sitter parsing and graph extraction
├── watcher/         File system event watching
├── store/           SQLite database (the only module that touches DB)
├── security/        Taint analysis, OWASP, SBOM
├── memory/          Cross-session observation management
├── bundle/          JSON export/import for team sharing
├── agents/          IDE/agent auto-configuration
└── cli/             Subcommand definitions
```

## Code style

- Run `cargo fmt` before committing
- Run `cargo clippy` and fix all warnings
- Follow the dependency rules in `.agent/instructions.md`: only `store/` may open database connections

## Submitting changes

1. Fork the repository
2. Create a feature branch: `git checkout -b my-feature`
3. Make your changes with tests
4. Run the full test suite: `cargo test`
5. Run formatting and lints: `cargo fmt && cargo clippy`
6. Commit with a clear message describing what and why
7. Push to your fork and open a pull request

## Pull request guidelines

- Keep PRs focused on a single change
- Include tests for new functionality
- Update documentation if behavior changes
- Reference any related issues in the PR description

## Adding language support

To add a new tree-sitter language:

1. Add the grammar crate to `Cargo.toml`
2. Add the grammar compilation to `build.rs`
3. Create `src/indexer/languages/{language}.rs` with extraction logic
4. Add file extension mapping in `src/indexer/parser.rs`
5. Add fixture tests in `tests/unit/parser/`
6. Update the language table in `docs/architecture.md` and `README.md`

## Pre-release verification

Before cutting a release, run the release readiness checks:

```sh
node scripts/verify-release.js
```

This verifies:
- Archive naming consistency across the release workflow, install.js, and install.sh
- Documentation link integrity (all relative links resolve to existing files)
- Version string consistency across Cargo.toml, npm/package.json, and CHANGELOG.md

All checks must pass before tagging a release.

## Reporting issues

- Use GitHub Issues for bug reports and feature requests
- Include Cortex version (`cortex --version`), OS, and reproduction steps
- For crashes, include the full error output

## Security

See [SECURITY.md](SECURITY.md) for reporting security vulnerabilities.
