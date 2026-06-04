# Changelog

All notable changes to Cortex are documented here.

## [1.1.0] - 2026-06-01

### Added

**SCIP Index Ingestion**
- Parses SCIP protobuf index files (`.scip/index.scip`, `index.scip`, `dump.lsif`) and creates HIGH-confidence edges (confidence=1.0, edge_source=`scip`)
- Supports indexes produced by scip-python, scip-typescript, rust-analyzer, scip-java, and scip-go
- SCIP edges win over tree-sitter edges on conflicts — deduplication keeps SCIP as primary, tree-sitter as secondary
- Additive, non-blocking pass after tree-sitter extraction; malformed SCIP files log a warning and fall back gracefully
- `cortex status` shows SCIP coverage percentage per language
- `cortex index --generate-scip` suggests the appropriate indexer for the detected primary language when no SCIP index is present

**Confidence-Tagged Edges**
- Every edge now carries `edge_source` (`scip`, `framework_adapter`, `ast_direct`, `name_match`) and `confidence` (1.0 / 0.8 / 0.5 / 0.3)
- All MCP tools include edge_source and confidence_tier in results so agents can filter by reliability
- Default query threshold: confidence >= MEDIUM (0.7) — filters out name-match heuristics by default
- Optional `min_confidence` parameter on `trace_callers` and `blast_radius` to lower or raise the threshold

**Framework Adapters** — six new adapters detect wiring that tree-sitter cannot see:
- **FastAPI**: `Depends(X)` → `Injects` edges; `@app.get/post/router.get/post/put/delete` → `Routes` edges; transitive dependency chains
- **Express**: `app.use(middleware)` / `router.use(middleware)` → `Middleware` edges; `router.get/post/put/delete(path, handler)` → `Routes` edges
- **NestJS**: `@Controller()` → `Routes` edges; `@Injectable()` constructor injection → `Injects` edges
- **Spring**: `@Autowired` / `@Inject` → `Injects` edges; `@Component/@Service/@Repository/@Controller` marks injectable; `@Bean` factory methods
- **Django**: `urlpatterns path()/re_path()` → `Routes` edges; `@login_required/@permission_required` → `Middleware` edges
- **React**: JSX component renders → `Renders` edges; `useContext(SomeContext)` → `Injects` edges
- Framework detection scans dependency manifests (`package.json`, `requirements.txt`, `pyproject.toml`, `pom.xml`, `build.gradle`, `go.mod`, `Gemfile`, `composer.json`) — only relevant adapters run
- Manual override via `.cortex/config.toml` `frameworks = ["fastapi", "express"]`

**User-Configurable Pattern Rules**
- `.cortex/patterns.toml` supports custom regex rules with named capture groups, edge kind, and confidence tier
- Enables framework-specific wiring for any framework not covered by built-in adapters

**Evidence-Fusion Task Context** (`get_task_context` overhaul)
- Multi-signal ranking: lexical match (BM25, 0.30) + embedding similarity (0.25) + SCIP reference distance (0.20) + git recency (0.15) + edge confidence (0.10) + file size penalty (−0.05)
- Weight redistributed to lexical (0.55) when embeddings are unavailable
- Greedy budget packing always includes top-1 result; each included file gets a one-line reason explaining selection
- Falls back to file-proximity heuristic (recently modified files matching keywords) when graph signals are weak
- Response includes `confidence` (0.0–1.0) and `coverage_percent` fields
- Non-empty results guaranteed on a healthy index

**Ask Tool Overhaul**
- Priority-ordered data source querying: SCIP edges first → framework adapter edges → AST-direct edges → grep fallback
- `FallbackSuggestion` returned when average confidence < 0.7 or zero results — includes grep commands and file-read suggestions derived from query terms
- Token cap: results capped at 1000 tokens when average confidence is below MEDIUM
- `confidence_warning` annotation on responses when results exist but confidence is low (never returns empty when results are available)
- Detects unindexed file paths in queries and returns targeted fallback suggestions

**Index Health Gate**
- Startup health check validates `files_indexed > 0`, `node_count > 0`, `edge_count > 0`
- All MCP tools return a structured `HealthError` (with reason, suggested action, and fallback grep commands) when the index is unhealthy — never serves confidently wrong data
- `cortex_status` tool is exempt from the health gate
- `cortex index --repair` rebuilds the index from scratch (clears all graph tables, resets FTS5, re-runs full pipeline) while preserving observations and ADRs

**Token Savings Accounting** (honest math)
- `baseline_cost` computed as `(matching_file_count × avg_file_tokens) + grep_output_tokens` using FTS5 to count matching files
- `net_saved = baseline_cost − (cortex_response_tokens + query_overhead_tokens)` — negative values reported without modification
- `cortex status --savings` displays: cumulative net saved, average per query, net-negative query count, baseline total vs actual total

**Repo Brief** (`get_repo_brief` — new MCP tool)
- Zero-parameter cold-start summary under 400 tokens: detected languages and frameworks, top 5 entry points, top 10 hotspots (complexity × churn), auth/security patterns, test coverage shape, index health
- Cached in `repo_brief_cache` table; invalidated when `files_indexed`, `node_count`, or `edge_count` changes
- Returns partial info even when the index is unhealthy

**Tool Surface Management**
- Default tools (always-on, 10): `get_repo_brief`, `get_task_context`, `ask`, `trace_callers`, `blast_radius`, `get_complexity_hotspots`, `get_git_hotspots`, `search_symbols`, `write_observation`, `read_observations`
- Experimental tools (opt-in via `.cortex/config.toml` `experimental_tools = true`, 7): `find_taint_paths`, `check_dependencies`, `decompose_boundaries`, `generate_steering`, `find_dead_code`, `generate_sbom`, `find_similar_functions`
- Smart-tools mode (`cortex serve --smart-tools`, 5): `get_repo_brief`, `ask`, `get_task_context`, `write_observation`, `read_observations`
- All tool descriptions kept under 100 tokens
- `semantic_search` only appears in the tool manifest when embeddings are built

**Hybrid Semantic Retrieval**
- Incremental embedding: only re-embeds functions whose content hash changed since the last run; removes stale embeddings for deleted nodes
- `cortex semantic enable` builds the initial embedding index (Ollama nomic-embed-code or bundled ONNX model)
- Cosine similarity integrated as a 0.25-weight signal in evidence-fusion ranking
- `cortex status` shows embedding count and a degradation warning when running in BM25-only mode

**Correctness Benchmarks**
- `cortex benchmark` command runs JSON-based test suites with ground-truth answers for `trace_callers`, `blast_radius`, `get_task_context`, and `ask`
- Reports pass rate, per-tool accuracy, and average token savings vs grep baseline
- 24-case self-referential benchmark suite ships in `benchmark/cortex_self_benchmark.json`
- CI gate: `cortex benchmark` runs on every release and exits with code 1 if pass rate drops below 70%
- `cortex status` displays a warning when the last benchmark run was below the 70% threshold

**Schema Migrations**
- `0009_confidence_edges.sql`: adds `edge_source` and `confidence` columns to edges; adds new edge kinds (`Injects`, `Middleware`, `Routes`, `Renders`); migrates existing edges to `ast_direct` / 0.5
- `0010_scip_coverage.sql`: `scip_coverage` table (per-file SCIP tracking) and `index_health` singleton table
- `0011_enhanced_savings.sql`: adds `baseline_cost`, `net_saved`, `query_terms` columns to `token_savings`
- `0012_repo_brief_cache.sql`: `repo_brief_cache` table for cached repo briefs

### Changed
- `trace_callers` now traverses `Injects` edges (framework DI injection sites) and follows `Implements` edges to include callers of interface/trait methods; annotates each result with `edge_source` and `confidence_tier`
- `blast_radius` now traverses all edge types (`Calls`, `Imports`, `Injects`, `Implements`, `Middleware`, `Routes`, `Renders`) with confidence filtering
- `get_task_context` replaced with evidence-fusion ranking (see above); response format extended with `confidence`, `coverage_percent`, and `reasons` fields
- `cortex status` extended with SCIP coverage %, active framework adapters, per-language file breakdown, semantic search status, and benchmark pass rate warning
- Indexing pipeline now runs framework detection before adapters, SCIP ingestion after tree-sitter, and updates `index_health` at the end of every index run
- Universal exclusions hardened: generation markers (`// Code generated`, `# AUTO-GENERATED`, `@generated`) detected in first 2KB; `*.map` source maps excluded; `.cortexignore` support verified

### Fixed
- `get_task_context` no longer returns empty on a healthy index — file-proximity fallback guarantees at least one result
- `ask` no longer returns confidently wrong answers from a corrupt or empty index — health gate blocks all tools when unhealthy
- `trace_callers` missed framework-wired connections (FastAPI `Depends`, Spring `@Autowired`, NestJS constructor injection) — now included via `Injects` edge traversal
- Low-confidence name-match edges no longer pollute results by default — filtered at confidence >= 0.7
- User-defined pattern rules (`.cortex/patterns.toml`) were incorrectly stored with `edge_source=name_match` (confidence 0.3) instead of `edge_source=framework_adapter` (confidence 0.8) — pattern rule edges now pass the default `min_confidence >= 0.7` filter and appear correctly in `trace_callers`, `blast_radius`, and `ask` results

## [1.0.4] - 2026-05-25

### Added
- `cortex uninstall` command: removes all Cortex traces (binary, graph DB, config, PATH entries, steering files)
- Installer automatically removes stale cortex binaries from `~/.local/bin`, npm global, and other known locations before installing
- Installer prepends `~/.cortex/bin` to the current session PATH so reindex uses the new binary immediately
- Installer checks `.profile` in addition to `.bashrc`/`.zshrc` for Unix PATH configuration
- Live visualizer (`cortex serve` at `http://127.0.0.1:9749`) now serves the clean viz.rs-style 3D graph UI
- Dashboard page at `/dashboard` with token savings, symbol search, and tool usage table
- Navigation between graph view and dashboard via header links
- `Method` node kind added to SQLite CHECK constraint (fixes indexing of Python `__init__`, class methods, TS constructors)
- CI auto-fixes formatting and clippy before checking (no more spurious failures)

### Fixed
- `get_architecture` files count now derived from distinct files in the nodes table (fixes inaccurate count when `files` table drifts from actual indexed content)
- Old cortex binaries in `~/.local/bin` or npm global shadowing the new install (caused `cortex -V` showing wrong version after update)
- IDE auto-detection no longer falsely detects agents from generic directories (`.github/`, `.vscode/`, `.idea/`, `.kiro/`)
- Embedded migrations 0007 and 0008 were missing from the binary (Method kind, token savings tables)
- Windows `setx` PATH note tells user to open a new terminal

### Changed
- HTML templates moved from `src/cli/commands/` to `src/cli/templates/` (proper structure)
- Visualizer root (`/`) serves the viz.rs-style graph; dashboard at `/dashboard`
- Dashboard rewritten: clean dark UI matching viz.rs aesthetic, no more janky tabbed interface
- Removed dead `dashboard_html.html` file

### Removed
- Old tabbed unified UI (replaced by separate graph + dashboard pages)

## [1.0.3] - 2026-05-25

### Added
- `cortex update` self-update command: downloads latest release from GitHub, verifies SHA-256 checksum, replaces binary (with Windows rename-then-replace pattern), and triggers reindex
- `cortex reindex` command: deletes and rebuilds the graph database from scratch
- Default exclusions for `.serena`, `.cursor`, `.kiro`, `.agent` directories and lock files (`pnpm-lock.yaml`, `package-lock.json`, `yarn.lock`, `Cargo.lock`)
- Renamed `.cortex-ignore` to `.cortexignore` for consistency
- Installer PATH configuration (Windows `setx`, Unix shell profile export)
- Post-install automatic reindex with 120s timeout
- Windows x86 (`win32-ia32`) binary support in release workflow and npm installer

### Changed
- Visualizer serves dynamic graph template (fetches `/api/nodes` and `/api/edges` at runtime) with loading state, reset button, and additional node kind badges (Method, Interface, Type)
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
