# Cortex

[![Release](https://img.shields.io/github/v/release/1337Xcode/cortex?style=flat-square&color=blue)](https://github.com/1337Xcode/cortex/releases)
[![Build](https://img.shields.io/github/actions/workflow/status/1337Xcode/cortex/release.yml?style=flat-square&label=build)](https://github.com/1337Xcode/cortex/actions)
[![License](https://img.shields.io/badge/license-MIT-green?style=flat-square)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![MCP Tools](https://img.shields.io/badge/MCP%20tools-32-purple?style=flat-square)](docs/tools.md)
[![Languages](https://img.shields.io/badge/languages-29-teal?style=flat-square)](docs/architecture.md)
[![Platform](https://img.shields.io/badge/platform-linux%20%7C%20macos%20%7C%20windows-lightgrey?style=flat-square)]()
[![npm](https://img.shields.io/npm/v/@1337xcode/cortex?style=flat-square&color=red&label=npm)](https://www.npmjs.com/package/@1337xcode/cortex)

One binary. Zero dependencies. 29 languages. 32 MCP tools. Local code intelligence for AI coding agents.

## Why Cortex

AI agents burn tokens reading files to answer structural questions. An agent asking "what calls processOrder" gets a 200-token graph result instead of burning 20,000 tokens reading files. Cortex indexes your repository into a SQLite call graph and exposes it over the Model Context Protocol. It handles repositories with 100K to 1M+ lines of code without breaking a sweat. Indexes 127 files in 535ms, incremental re-index in 13ms.

## Install

**npx (recommended)**

```sh
npx @1337xcode/cortex install
```

**Shell script**

```sh
curl -fsSL https://raw.githubusercontent.com/1337Xcode/cortex/main/install.sh | sh
```

**Build from source**

```sh
git clone https://github.com/1337Xcode/cortex.git && cd cortex
cargo build --release
cp target/release/cortex ~/.local/bin/
```

## Quick start

```sh
cortex index    # parse your repo, build the graph
cortex install  # detect Claude Code / Cursor, write MCP config
cortex serve    # start the MCP server
```

Your agent can now query the graph instead of reading raw files.

## What you get

- **29 languages** parsed via tree-sitter (Python, TypeScript, Rust, Go, Java, C#, C++, Ruby, Kotlin, Swift, PHP, Scala, and 17 more)
- **32 MCP tools** exposed over stdio, or 5 in smart-tools mode (the `ask` meta-tool routes internally)
- **Sub-second incremental re-indexing** via native OS file watcher (inotify, FSEvents, ReadDirectoryChangesW)
- **Taint flow analysis, OWASP Top 10 detection, SBOM generation** all running locally, no cloud
- **Cross-session memory** that marks observations stale when linked code changes
- **Multi-repo federation** for querying across repositories with a unified graph
- **Hybrid search** combining FTS5 BM25 with optional local ONNX vector search
- **CI quality gates** with configurable thresholds and exit codes
- **3D interactive graph visualization** exportable to standalone HTML
- **Portable JSON bundle** (cortex.json) for team sharing via git
- **Works offline.** No cloud, no API keys, no Docker, no language runtimes.

## Demo commands

```sh
cortex impact UserService.getUser       # blast radius: what breaks if I change this?
cortex explain DatabasePool.acquire     # offline function explanation, zero LLM calls
cortex security report                  # taint flows, OWASP patterns, dependency vulns
cortex diff main feature-branch         # call graph diff between branches
cortex viz --export graph.html          # interactive 3D graph in a standalone HTML file
cortex ci --fail-on-taint               # CI quality gate with exit codes
cortex hotspots --months 6              # high-churn risky code ranked by risk score
cortex coverage --lcov coverage.lcov    # untested functions ranked by caller count
cortex modules                          # Leiden community detection module boundaries
cortex federate add ../auth-service     # query across repos with unified graph
```

## Architecture

```mermaid
graph TD
    A[Repository Files] --> B[File Watcher<br/>notify crate]
    B --> C[Indexer Pipeline<br/>Rayon + tree-sitter]
    C --> D[SQLite Graph Store<br/>WAL + FTS5 + sqlite-vec]
    D --> E[MCP Server<br/>Tokio + JSON-RPC 2.0]
    E --> F[AI Coding Agent]

    C --> G[Security Pass<br/>Taint + OWASP + SBOM]
    G --> D

    D --> H[Bundle Exporter<br/>cortex.json]
    H --> I[Git Repository<br/>Team Sharing]

    D --> J[Memory Layer<br/>Staleness-aware observations]
```

## Comparison

| Feature | Cortex | codebase-memory-mcp | Srclight | Serena | mcp-codebase-index |
|---------|--------|---------------------|----------|--------|-------------------|
| Architecture | Rust binary, SQLite | C binary, SQLite | Python, SQLite | Python, LSP | Python, SQLite |
| Languages | 29 (tree-sitter) | 66 (tree-sitter) | 10 (tree-sitter) | 40+ (LSP) | 4 |
| MCP tools | 32 (or 5 smart mode) | 14 | 42 | ~20 | 17 |
| Smart tool routing | Yes (ask meta-tool) | No | No | No | No |
| Call graph | Function-level | Full chains | Callers/callees | LSP-precise | Cross-file |
| Hybrid search | FTS5 + sqlite-vec | Graph only | RRF fusion | Keyword | Structural |
| Token reduction | 10-100x | 121x avg | Not measured | Not measured | 87% |
| Incremental update | 13ms (no changes) | Milliseconds | Git hooks | LSP live | Hash-based |
| Security (taint/OWASP/SBOM) | Yes | No | No | No | No |
| Cross-session memory | Staleness-aware | No | No | No | No |
| Multi-repo federation | Yes | No | SQLite ATTACH | Partial | No |
| Document ingestion | Yes | No | No | No | No |
| Build system awareness | Cargo, npm, Go, Gradle, Maven | No | Yes | No | No |
| Git intelligence | Hotspots, churn | Commit-aware delta | git blame | No | No |
| Community detection | Leiden algorithm | No | Leiden | No | No |
| Coverage gap analysis | Yes (LCOV cross-ref) | No | No | No | No |
| CI quality gates | Yes (exit codes) | No | No | No | No |
| Single binary, zero deps | Yes | Yes | No (Python) | No (Python+LSP) | No (Python) |
| Auto IDE config | 2+ agents | No | No | No | No |
| 3D visualization | Yes | Yes | No | No | No |
| Air-gap compatible | Yes | Yes | Yes | Yes | Yes |
| License | MIT | MIT | MIT | MIT | MIT |

## Documentation

- [Getting started](docs/getting-started.md)
- [IDE setup](docs/ide-setup.md)
- [CLI reference](docs/cli-reference.md)
- [MCP tools](docs/tools.md)
- [Configuration](docs/configuration.md)
- [Architecture](docs/architecture.md)

## Website

The documentation site is built with Astro 5 and deployed to GitHub Pages.

```sh
cd site
npm install
npm run dev       # local dev at http://localhost:4321
npm run build     # production build to dist/
```

Live at [1337xcode.github.io/cortex](https://1337xcode.github.io/cortex)

## License

[MIT](LICENSE)
