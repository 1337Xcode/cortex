---
title: "CLI Reference"
description: "Complete reference for all Cortex CLI commands and options."
order: 3
category: "reference"
lastModified: "2025-01-15"
---

# CLI reference

Complete reference for all `cortex` commands.

## cortex status

Print graph statistics for the current repository.

```sh
cortex status
```

Shows: node count, edge count, file count, languages detected, last index time, database size.

## cortex index

Parse and index the current repository.

```sh
cortex index
```

Walks the repository, parses each file with tree-sitter, extracts symbols and call edges, stores them in SQLite. Skips files whose content hash has not changed since the last run.

Environment variables:
- `CORTEX_REPO_ROOT`: override the repository root (defaults to current directory)

## cortex serve

Start the MCP server.

```sh
cortex serve
cortex serve --smart-tools
```

Starts the MCP server over stdio (JSON-RPC 2.0). Also starts the file watcher for incremental re-indexing. Logs go to stderr (stdout is reserved for MCP protocol).

Options:
- `--smart-tools`: only expose 5 core tools (ask, search_symbols, write_observation, read_observations, get_architecture) to reduce context window overhead. The `ask` meta-tool handles routing internally.

## cortex install

Auto-detect and configure AI agents.

```sh
cortex install
cortex install --platform cursor
```

Scans for installed agents, writes MCP server config for each one found. Also writes a workspace-level `.cortex/mcp.json` for VS Code/Cursor/Kiro auto-discovery.

Options:
- `--platform <name>`: configure a specific agent instead of auto-detecting

Supported platforms: claude-code, cursor, windsurf, vscode, kiro, zed, jetbrains, cline, continue, aider

## cortex security

Security analysis commands.

### cortex security scan

```sh
cortex security scan
```

Runs taint flow analysis and OWASP pattern detection. Outputs structured JSON with findings.

### cortex security sbom

```sh
cortex security sbom
```

Generates an SPDX 2.3 SBOM from the import graph. Extracts dependencies from lock files (Cargo.lock, package-lock.json, go.sum, requirements.txt).

### cortex security report

```sh
cortex security report
```

Prints a human-readable security summary to stdout. Includes taint flow count, OWASP patterns by category, and dependency vulnerability status.

### cortex security vulns

```sh
cortex security vulns
```

Cross-references SBOM entries against OSV.dev for known CVEs. Requires network access.

## cortex report

Generate a codebase report.

```sh
cortex report
cortex report --output /tmp/report.md
```

Writes CORTEX_REPORT.md with: architecture overview, most-connected functions, dead code candidates, security findings, and Leiden community boundaries.

Options:
- `--output`, `-o`: output path (defaults to CORTEX_REPORT.md in repo root)

## cortex impact

Show what breaks if you change a function.

```sh
cortex impact src/db.rs::connect
cortex impact --depth 5 UserService.getUser
```

Computes blast radius: all functions that transitively depend on the given symbol. Groups results by file.

Options:
- `--depth`: maximum traversal depth (default 4)

## cortex explain

Offline function explanation.

```sh
cortex explain DatabasePool.acquire
```

Prints: kind, file location, line count, callers, callees, security flags, and any stored observations. Zero LLM calls.

## cortex diff

Compare call graphs between branches.

```sh
cortex diff main feature-branch
```

Shows files changed between branches, symbols in those files, and downstream blast radius.

## cortex viz

Interactive graph visualization.

```sh
cortex viz --export graph.html
cortex viz --port 9749
```

Options:
- `--export <path>`: generate a standalone HTML file with embedded 3D graph
- `--port`: port for the visualization server (default 9749)

## cortex ci

CI quality gate.

```sh
cortex ci --fail-on-taint --fail-on-owasp
cortex ci --fail-on-dead-code-above 15 --format json
```

Runs all analyses and exits non-zero if thresholds are exceeded. Designed for CI pipelines.

Options:
- `--fail-on-taint`: exit 1 if any taint flows detected
- `--fail-on-owasp`: exit 1 if any OWASP patterns detected
- `--fail-on-dead-code-above <pct>`: exit 1 if dead code percentage exceeds threshold
- `--format`: output format, "text" (default) or "json"

## cortex clone

Clone and index a remote repository.

```sh
cortex clone https://github.com/torvalds/linux
cortex clone git@github.com:user/repo.git --dir ./my-copy
```

Runs `git clone --depth 1` then `cortex index` on the result.

Options:
- `--dir`: local directory to clone into (defaults to repo name extracted from URL)

## cortex hook

Manage git hooks.

### cortex hook install

```sh
cortex hook install
```

Installs a post-commit hook that runs `cortex index` in the background after each commit.

### cortex hook remove

```sh
cortex hook remove
```

Removes the Cortex section from the post-commit hook.

### cortex hook status

```sh
cortex hook status
```

Shows whether the Cortex hook is installed.

## cortex modules

Show detected module boundaries using Leiden community detection.

```sh
cortex modules
cortex modules --threshold 0.3
```

Prints clusters of tightly-coupled code with their core functions, coupling scores, and file membership. Useful for understanding module boundaries and identifying refactoring candidates.

Options:
- `--threshold`: coupling threshold for community detection, 0.0 to 1.0 (default 0.5). Lower values produce more granular clusters.

## cortex query

Graph query subcommands.

### cortex query callers

```sh
cortex query callers src/main.rs::main --depth 5
```

### cortex query callees

```sh
cortex query callees src/db.rs::connect --depth 2
```

### cortex query find

```sh
cortex query find "process*" --kind Function --limit 10
```

### cortex query architecture

```sh
cortex query architecture
```

### cortex query dead-code

```sh
cortex query dead-code --limit 100
```

### cortex query blast-radius

```sh
cortex query blast-radius src/auth.rs::verify_token --depth 4
```

### cortex query changes

```sh
cortex query changes --since 1715000000
```

## cortex bundle

Bundle operations.

### cortex bundle export

```sh
cortex bundle export
cortex bundle export --format ccg
```

### cortex bundle import

```sh
cortex bundle import
cortex bundle import /path/to/cortex.json
```

## cortex semantic

Semantic search management.

### cortex semantic enable

Downloads the nomic-embed-text-v1 ONNX model (138 MB) and enables vector search.

### cortex semantic disable

Removes the model and disables vector search.

### cortex semantic status

Reports whether semantic search is available.

## cortex memory

### cortex memory list

Lists all stored observations.

### cortex memory prune --stale

Removes observations marked as stale.

### cortex memory show

```sh
cortex memory show
```

Displays all cross-session observations and ADRs stored in the database. Observations are grouped by symbol FQN, with staleness indicators and relative timestamps. ADRs are listed with their status, linked symbol, and creation time.

Output includes:
- Active, stale, and archived observation counts
- Observations grouped by the symbol they describe
- ADR title, status, linked FQN, and summary

## cortex coverage

Cross-reference call graph with test coverage data.

```sh
cortex coverage --lcov coverage.lcov
cortex coverage --lcov lcov.info --limit 50
```

Reads an LCOV coverage file and cross-references covered/uncovered lines with the call graph. Outputs a ranked list of untested functions sorted by how many other functions call them (highest-risk coverage gaps first).

Options:
- `--lcov`: path to LCOV coverage file (default: coverage.lcov)
- `--limit`: maximum number of results to show (default: 30)

Output columns: Function, File, Line, Callers, Coverage Status.

Supports LCOV files generated by: cargo-tarpaulin, jest, pytest-cov, gcov, llvm-cov, istanbul, and any tool that produces standard LCOV format.

## cortex hierarchy

Print class inheritance and interface implementation tree.

```sh
cortex hierarchy UserService
cortex hierarchy com.example.UserViewController
```

Takes a class or type FQN and prints the full hierarchy tree: parent classes (what it extends), interfaces (what it implements), children (what extends it), and implementors (what implements it, if it is an interface).

Works for Kotlin, Swift, Dart, Java, C#, and TypeScript classes. If the exact FQN is not found, suggests fuzzy matches from the graph.

## cortex hotspots

Find high-churn, high-complexity code.

```sh
cortex hotspots
cortex hotspots --months 6 --limit 20
```

Combines git commit frequency with call graph connectivity to identify the riskiest code in the repository. Files that change often and have many callers represent the highest maintenance risk.

Risk score formula: `churn_count * caller_count` (higher = more maintenance risk).

Options:
- `--months`: how far back in git history to look (default 6)
- `--limit`: maximum number of results to show (default 30)

Output columns: Risk score, Churn (commit count), Callers (inbound call edges), Function FQN and file path.

Requires git to be installed and the command to be run inside a git repository with commit history.

## cortex federate

Multi-repo federation management.

### cortex federate add

```sh
cortex federate add ../auth-service
cortex federate add /absolute/path/to/other-repo
```

Adds a repository to the federation. The target must have a `.cortex/` directory (run `cortex index` there first). Federated queries search across all member repos.

### cortex federate list

```sh
cortex federate list
```

Shows all federated repositories with their names, paths, and accessibility status.

### cortex federate remove

```sh
cortex federate remove auth-service
```

Removes a repository from the federation by name.

## cortex ingest

Ingest documents into the knowledge graph.

```sh
cortex ingest ./docs
cortex ingest ./specs --types md,txt,yaml
cortex ingest ./data/report.csv
```

Walks the given path and ingests supported file types as Document nodes in the graph. Documents are linked to code nodes when they mention known function or class names.

Supported file types: md, txt, csv, rst, html, yaml, json, pdf (reference node only, no text extraction from PDF binary content).

Options:
- `--types`: comma-separated list of file extensions to include (default: md,txt,csv,rst,html,yaml)

## cortex modules

Show detected module boundaries.

```sh
cortex modules
cortex modules --threshold 0.3
cortex modules --build-system
```

Without `--build-system`: runs Leiden community detection on the call graph and prints clusters of tightly-coupled code with their core functions and file membership.

With `--build-system`: detects the project's build system (Cargo, npm, Go, Gradle, Maven) and prints workspace members with their paths.

Options:
- `--threshold`: coupling threshold for community detection, 0.0 to 1.0 (default 0.5)
- `--build-system`: show workspace members from the build config instead of graph-detected modules

## cortex config

Manage Cortex configuration.

### cortex config get

```sh
cortex config get pool_size
cortex config get repo_root
```

Reads a configuration value from .cortex/config.toml or environment variables.

### cortex config set

```sh
cortex config set pool_size 8
cortex config set log_level debug
```

Writes a configuration value to .cortex/config.toml.

### cortex config reset

```sh
cortex config reset
```

Resets configuration to defaults by removing .cortex/config.toml.

## Environment variables

| Variable | Description | Default |
|----------|-------------|---------|
| CORTEX_REPO_ROOT | Repository root path | Current directory |
| CORTEX_DATA_DIR | Data directory for graph.db | .cortex/ in repo root |
| CORTEX_LOG_LEVEL | Log level (trace, debug, info, warn, error) | info |
| CORTEX_POOL_SIZE | SQLite read connection pool size | 4 |
