# Changelog

All notable changes to Cortex are documented here.

## [1.0.3] - 2026-06-01

### Added
- `cortex update` self-update command: downloads latest release from GitHub, verifies SHA-256 checksum, replaces binary (with Windows rename-then-replace pattern), and triggers reindex
- `cortex reindex` command: deletes and rebuilds the graph database from scratch
- Default exclusions for `.serena`, `.cursor`, `.kiro`, `.agent` directories and lock files (`pnpm-lock.yaml`, `package-lock.json`, `yarn.lock`, `Cargo.lock`)
- Renamed `.cortex-ignore` to `.cortexignore` for consistency
- Installer PATH configuration (Windows `setx`, Unix shell profile export)
- Post-install automatic reindex with 120s timeout
- Windows x86 (`win32-ia32`) binary support in release workflow and npm installer

### Changed
- Unified UI fully dark-themed (`#1e1e2e` background) with corner-positioned icon navigation replacing the nav bar
- Hotspots table uses `overflow-x: auto` to prevent horizontal overflow
- Statistics overlay uses CSS Grid with `tabular-nums` for numeric alignment

## [1.0.2] - 2026-05-25

### Added
- Configurable model pricing via `~/.cortex/pricing.toml` with longest-prefix matching
- Ego-graph node cap at 500 with priority ordering (depth ASC, caller_count DESC)
- Coverage field on graph nodes populated from LCOV data
- Agent steering improvements: module boundaries, complexity hotspots, active ADRs
- Unified tabbed UI (Graph, Dashboard, Explorer) served at visualizer root
- `GET /api/metrics` and `GET /api/symbols` endpoints for the visualizer
- Port documentation in `docs/ports.md`
- Property-based tests for version comparison, pricing, ego-graph, coverage, NodeKind, steering, install, and release notes
- Update notification on startup when newer version available

### Changed
- CI pipeline now fails on clippy warnings and formatting violations
- Release workflow publishes to npm automatically after GitHub Release
- npm installer supports single-command install and update via `npx @1337xcode/cortex install`
- IDE install hardened with config validation, directory creation, and permission error reporting
- Method vs Function NodeKind correctly assigned across Python, TypeScript, Rust, Go, Java
- Steering generation enforces 2000-token budget

### Removed
- Obsolete files (site/_patch_mcp.py)
- `continue-on-error: true` from CI clippy and fmt steps
- Stale copy style inconsistencies across source and documentation

## [1.0.1] - 2026-05-19

### Added
- Support for 25 AI coding agents (up from 15): added OpenCode, OpenClaw, Factory Droid,
  Trae, Trae CN, Gemini CLI, Hermes, Kimi Code, Kiro IDE, and Pi coding agent.
- Dedicated subcommands: `cortex cursor install`, `cortex vscode install`,
  `cortex kiro install`, `cortex antigravity install` for one-step setup without `--platform`.
- Platform alias normalization: `--platform` now accepts flexible names
  (e.g. `copilot`, `codex`, `droid`, `claw`, `trae-cn`) and maps them to canonical IDs.
- `synthesize_agent` fallback: `cortex install --platform <name>` now works even when
  the agent's config directory does not exist yet (creates it on the fly).
- Comprehensive IDE setup documentation with quick-reference install table.
- `.editorconfig` at repo root to enforce UTF-8 without BOM, LF line endings, and
  consistent indentation across all editors going forward.
- CI `lint-scripts` job that fails the build if any `.js`, `.ts`, `.mjs`, `.cjs`, or
  `.sh` file contains a UTF-8 BOM, preventing regressions.

### Fixed
- Stripped UTF-8 BOM (EF BB BF) from `npm/scripts/install.js` that caused a Node.js
  `SyntaxError: Invalid or unexpected token` on `npm install`, breaking the post-install
  binary download on all platforms.

### Changed
- Antigravity renamed to "Google Antigravity" in display output.
- Codex CLI detection now also checks `~/.codex/` (home directory).
- `cortex install` help output lists all 25 supported platforms with config file paths.

## [1.0.0] - 2026-05-18

### Added
- Documentation site built with Astro 5, deployed to GitHub Pages
- Interactive 3D codebase visualization (same as `cortex viz` output)
- Dark/light mode with system preference detection
- Bento grid feature showcase with animated cards
- Ctrl+K / Cmd+K search across docs
- GitHub OAuth issue submission form via Cloudflare Worker
- RSS and Atom feeds for documentation updates
- `llms.txt` and `ai-plugin.json` for AI discoverability
- Confetti effect on install command copy

### Changed
- npm package version aligned with changelog (0.0.30)
- Site URL set to `1337xcode.github.io/cortex`
- Comparison table updated with accurate data from each project's docs

### Fixed
- Federation console animation now types top-to-bottom
- Bento card overflow on MCP tools and Federation cards
- Language marquee no longer pauses on hover
- Theme toggle properly switches between light and dark mode

## [0.0.29] - 2025-07-15

### Changed
- License changed from PolyForm Noncommercial to MIT
- Release workflow archive naming aligned with install scripts (darwin/x64/arm64/win32 convention)
- All platforms now produce tar.gz archives for consistency
- SECURITY.md version table corrected from 0.1.x to 0.0.x

### Added
- CONTRIBUTING.md with build, test, and PR guidelines

### Removed
- Committed binary files from npm/vendor/ (now download-only via GitHub Releases)
- Agent-specific files (skill/, SKILL.md, tile.json)

## [0.0.20] - 2025-05-17

### Changed
- All 32 MCP tools registered and verified in server.rs
- All stub CLI commands implemented: status, memory list, memory prune, security vulns, config get/set/reset
- Version synced to 0.0.20 across Cargo.toml, npm/package.json, SKILL.md, tile.json
- Documentation reconciled with actual implementation (no false claims remain)
- Removed non-existent --ui flag from docs

## [0.0.19] - 2025-05-16

### Added
- `cortex ask` MCP meta-tool: single-call code intelligence that auto-routes to the right internal tools and composes a unified answer
- `cortex federate add/remove/list`: multi-repo federation with unified cross-repo queries
- `cortex ingest <path>`: local document ingestion (markdown, text, CSV, HTML, YAML) into the knowledge graph
- `cortex serve --smart-tools`: expose only 5 core tools, reducing context window overhead by 89%
- Build system awareness: detects Cargo workspaces, npm workspaces, Go workspaces, Gradle/Maven multi-module projects
- `cortex hotspots`: combines git commit frequency with call graph connectivity to find maintenance risks
- `get_class_hierarchy`, `get_git_hotspots`, `get_import_graph`, `find_similar_functions` MCP tools
- `cortex coverage --lcov`: cross-references call graph with test coverage data

## [0.0.18] - 2025-05-12

### Added
- Leiden community detection algorithm for module boundary analysis
- `decompose_boundaries` MCP tool with coupling scores between clusters
- 3D graph visualization: `cortex viz --export graph.html` generates standalone HTML with embedded 3d-force-graph
- Nodes colored by community assignment, sized by caller count
- `cortex report` generates CORTEX_REPORT.md with architecture overview, hotspots, dead code, security findings

### Fixed
- Community detection was treating all edges as undirected; now respects call direction for modularity score

## [0.0.17] - 2025-05-10

### Added
- Hybrid search: when `search_symbols` returns fewer than 3 graph results, FTS5 BM25 runs as fallback
- Results merged and deduplicated by FQN, sorted by confidence descending
- `cortex semantic enable/disable/status` for local ONNX vector search management
- sqlite-vec compiled as a loadable extension, statically linked for HNSW vector search

### Changed
- FTS5 ranking switched to explicit BM25 weighting (k1=1.2, b=0.75)

## [0.0.16] - 2025-05-08

### Added
- Cross-session memory layer: `write_observation` stores text linked to a node FQN with agent ID and timestamp
- `read_observations` retrieves observations with `is_stale` boolean flag
- Staleness invalidation: when indexer detects a node's content hash changed, linked observations get `is_stale = true`
- `prune_observations` removes stale observations filtered by age
- ADR storage: `write_adr` / `read_adrs` with status and optional linked FQN
- Migration 0004 creates observations and adrs tables

## [0.0.15] - 2025-05-06

### Added
- `cortex security report` prints human-readable security summary with taint flows, OWASP categories, dependency count
- `check_dependencies` MCP tool cross-references SBOM entries against OSV.dev API
- Vulnerability check integrated into the report (skipped gracefully when offline)

### Changed
- SBOM generation extracts package versions from lock files (Cargo.lock, package-lock.json, go.sum, requirements.txt)

## [0.0.14] - 2025-05-04

### Added
- SBOM generation in SPDX 2.3 JSON format from the import graph
- `generate_sbom` MCP tool
- `cortex security sbom` CLI command
- Dependency extraction from Cargo.toml, package.json, go.mod, requirements.txt, pyproject.toml, Gemfile

## [0.0.13] - 2025-05-02

### Added
- OWASP Top 10 pattern detection against the structural call graph
- Patterns detected: A01 (Broken Access Control), A02 (Crypto Failures), A03 (Injection), A04 (Insecure Design)
- `scan_owasp` MCP tool returns findings with category, node FQN, and confidence
- Inter-procedural taint propagation: follows call edges up to depth 5

### Fixed
- Taint analysis was missing async function sinks in Python

## [0.0.12] - 2025-04-30

### Added
- Taint flow analysis: detects HTTP input sources flowing to SQL queries, file writes, shell command execution
- Source annotations for Flask, FastAPI, Express, Go net/http
- Sink annotations for raw SQL, os.system/subprocess, file open with write mode
- `find_taint_paths` MCP tool
- `cortex security scan` CLI command
- Migration 0003 creates security_findings and taint_paths tables

## [0.0.11] - 2025-04-28

### Added
- Bundle export: `cortex bundle export` serializes full graph to cortex.json
- Bundle import: `cortex bundle import` rebuilds SQLite from JSON bundle
- Bundle format versioned (schema_version field) for forward compatibility
- CCG export format via `cortex bundle export --format ccg`

## [0.0.10] - 2025-04-26

### Added
- `cortex install` command: scans for installed AI agents and writes MCP server config
- Detection for Claude Code and Cursor with idempotent config merging
- Expanded detection: Windsurf, VS Code, Zed, JetBrains (7 agents total)
- Workspace-level `.cortex/mcp.json` auto-written for VS Code/Cursor/Kiro auto-discovery

### Fixed
- Claude Code settings.json was being overwritten entirely instead of merged

## [0.0.9] - 2025-04-24

### Added
- HTTP route extraction for Python Flask/FastAPI, TypeScript Express, Go net/http
- `get_http_routes` and `trace_http_call` MCP tools
- Cross-service linking: when service A calls an endpoint matching service B's route, creates an edge

### Fixed
- Go parser was not extracting method receivers
- TypeScript arrow function exports were missing from the symbol table

## [0.0.8] - 2025-04-22

### Added
- 15 additional tree-sitter languages: Scala, Swift, PHP, SQL, Kotlin, Dart, Elixir, Haskell, Lua, Zig, Bash, Perl, R, Objective-C, OCaml
- Total language count now 25
- Language quality tiers: Tier 1 (Python, TS, Rust, Go, Java), Tier 2 (C#, C++, Ruby, Kotlin, Swift), Tier 3 (remaining)

### Changed
- Parser module refactored: one file per language under `src/indexer/languages/`

## [0.0.7] - 2025-04-19

### Added
- `query_graph` MCP tool: Cypher-like subset (MATCH, WHERE, RETURN, LIMIT, ORDER BY)
- `get_code_snippet` MCP tool: reads source lines for a symbol by FQN
- `detect_changes` MCP tool: nodes modified since a Unix timestamp
- `blast_radius` MCP tool: BFS over inbound edges to configurable depth
- MCP server over stdio transport (JSON-RPC 2.0, Tokio async runtime)
- `cortex serve` command with concurrent tool handling

## [0.0.6] - 2025-04-17

### Added
- File watcher using notify crate: inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows
- Sub-second incremental re-indexing: only re-parses files whose content hash changed
- .gitignore and .cortex-ignore exclusion rules applied to watcher events

### Fixed
- Watcher was triggering on .git/ internal file changes

## [0.0.5] - 2025-04-15

### Added
- MCP server initial tool set: `search_symbols`, `trace_callers`, `trace_callees`, `get_file_context`, `get_architecture`
- `find_dead_code` query: nodes with zero inbound call edges, excluding entry points
- FTS5 full-text search over symbol names, file paths, and FQN components

### Changed
- Schema migration system formalized: numbered SQL files applied in order on startup

## [0.0.4] - 2025-04-13

### Added
- Rayon parallel file parsing: each file gets its own tree-sitter parser instance on a thread pool
- Progress reporting during indexing (file count, elapsed time, files/second)

### Fixed
- Large repositories (50K+ files) caused OOM; now processed in batches of 500

## [0.0.3] - 2025-04-11

### Added
- Call edge extraction: function A calls function B creates a directed edge
- FQN resolution across files using the import graph
- Import relationship tracking stored as edges with kind "Imports"
- Two-pass resolution: first pass collects definitions, second pass resolves call targets

### Fixed
- Python nested function definitions were being skipped
- TypeScript re-exports were not creating import edges

## [0.0.2] - 2025-04-09

### Added
- SQLite store with WAL mode, nodes/edges/files tables
- tree-sitter parsing for 10 languages: Python, TypeScript, JavaScript, Go, Rust, Java, C#, C++, C, Ruby
- Symbol extraction: functions, classes, methods, modules, interfaces, enums
- `cortex index` walks the repository respecting .gitignore
- Migration 0001 creates initial schema with indexes

## [0.0.1] - 2025-04-05

### Added
- Initial project scaffold: Cargo.toml with clap, rusqlite, serde, tree-sitter
- CLI skeleton with `cortex index` and `cortex serve` (stub) subcommands
- Config loading from environment variables and .cortex/config.toml
- Structured logging via tracing crate with configurable log level
