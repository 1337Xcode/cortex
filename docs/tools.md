---
title: "MCP Tools"
description: "Documentation for all MCP tools exposed by the Cortex server."
order: 4
category: "reference"
lastModified: "2026-06-01"
---

# MCP tools

Cortex exposes tools over the Model Context Protocol. Every response includes `_meta` with token usage estimates.

## Tool surface

Tools are classified into three tiers. The active set is determined at server startup.

| Tier | Tools | How to enable |
|------|-------|---------------|
| **Default** (always-on) | `get_repo_brief`, `get_task_context`, `ask`, `trace_callers`, `blast_radius`, `get_complexity_hotspots`, `get_git_hotspots`, `search_symbols`, `write_observation`, `read_observations` | automatic |
| **Experimental** (opt-in) | `find_taint_paths`, `check_dependencies`, `decompose_boundaries`, `generate_steering`, `find_dead_code`, `generate_sbom`, `find_similar_functions` | `.cortex/config.toml` `experimental_tools = true` |
| **Smart-tools** (minimal) | `get_repo_brief`, `ask`, `get_task_context`, `write_observation`, `read_observations` | `cortex serve --smart-tools` |

`semantic_search` only appears in the manifest when embeddings are built (`cortex semantic enable`).

All tool descriptions are kept under 100 tokens to minimize system prompt overhead.

---

## Cold-start tool

### get_repo_brief

Zero-parameter cold-start summary of the entire codebase. Call this first when dropped into an unfamiliar repository — it replaces 10–20 exploration calls.

**Arguments:** none (zero-config)

**Response includes:**
- `languages`: detected programming languages (by file extension frequency)
- `frameworks`: detected frameworks from dependency manifests
- `entry_points`: top 5 entry points with one-line descriptions
- `hotspots`: top 10 hotspot files ranked by complexity × churn
- `security_patterns`: detected auth/security-related node FQNs
- `test_shape`: test directories and estimated function coverage percentage
- `health`: current index health status

**Token budget:** output kept under 400 tokens (character_count / 4).

**Caching:** result is cached and invalidated only when `files_indexed`, `node_count`, or `edge_count` changes. Returns partial info even when the index is unhealthy.

---

## Meta-tool

### ask

Single-call code intelligence. Pass a natural language question and Cortex auto-routes to the appropriate internal tools, composing a unified answer.

**Arguments:**
- `question` (required): natural language question about the codebase

**Routing logic:**
- "what calls X" / "who calls X" → trace_callers
- "what does X call" → trace_callees
- "what breaks if I change X" / "impact of X" → blast_radius
- "explain X" → callers + callees + observations
- "find X" / "where is X" → search_symbols
- "security" / "taint" / "vulnerability" → find_taint_paths + scan_owasp
- "dead code" / "unused" → find_dead_code
- "architecture" / "overview" → get_architecture
- fallback → search_symbols + search_text

**Data source priority:** SCIP edges first → framework adapter edges → AST-direct edges → grep-based fallback.

**Confidence behavior:**
- Results include `edge_source` and `confidence_tier` per item
- When average confidence < 0.7 (MEDIUM), a `FallbackSuggestion` is attached with grep commands and file-read suggestions
- Results are capped at 1000 tokens when average confidence is below MEDIUM
- `confidence_warning` is set when results exist but confidence is low — results are still returned (never empty when data exists)

**Response includes:** `summary` (intent, edge_sources, confidence, confidence_warning), `results`, `fallback` (optional FallbackSuggestion).

---

## Structural tools

### search_symbols

Find nodes by name pattern with optional kind filter.

**Arguments:**
- `pattern` (required): glob pattern to match against FQN (e.g., `*UserService*`, `src/auth*`)
- `kind` (optional): filter by node kind (Function, Class, Module, Route, Interface, Enum, Constant, Method)
- `limit` (optional): max results, default 50

**Behavior:** Searches the graph index first. If fewer than 3 results, automatically falls back to FTS5 BM25 search. Results are merged and deduplicated by FQN.

### trace_callers

BFS over inbound call edges. Answers "who calls this function?" Includes framework-wired injection sites (`Injects` edges) and callers of interface/trait methods (`Implements` edges).

**Arguments:**
- `fqn` (required): fully qualified name of the target node
- `depth` (optional): max traversal depth, default 3, max 5
- `min_confidence` (optional): minimum edge confidence threshold, default 0.7

**Returns:** array of `CallPathNode` with `fqn`, `kind`, `file`, `start_line`, `depth`, `confidence`, `call_count`, `edge_source`, `confidence_tier`. Sorted by call_count descending.

### trace_callees

BFS over outbound call edges. Answers "what does this function call?"

**Arguments:** same as trace_callers.

### get_file_context

Compressed structural summary of a file. Returns all symbols defined in it and their edges.

**Arguments:**
- `file` (required): relative file path

**Token savings:** approximately 500–800 tokens for a typical 300-line file vs 15,000+ tokens for the raw file content.

### get_architecture

High-level architecture summary. Languages, module structure, entry points, node/edge counts.

**Arguments:** none

### find_dead_code

Nodes with zero inbound call edges, excluding entry points (route handlers, main functions, test functions, framework decorators).

**Arguments:**
- `limit` (optional): max results, default 50

### blast_radius

Given a node FQN, returns all nodes that transitively depend on it. Traverses all edge types: `Calls`, `Imports`, `Injects`, `Implements`, `Middleware`, `Routes`, `Renders`.

**Arguments:**
- `fqn` (required): target node
- `depth` (optional): max traversal depth, default 3
- `min_confidence` (optional): minimum edge confidence threshold, default 0.7

**Returns:** array of `BlastRadiusNode` with `fqn`, `kind`, `file`, `start_line`, `depth`, `confidence`, `edge_source`, `confidence_tier`.

### detect_changes

Nodes modified since a given timestamp.

**Arguments:**
- `since` (required): Unix timestamp

### get_code_snippet

Returns the actual source lines for a symbol.

**Arguments:**
- `fqn` (required): target node

### query_graph

Cypher-like query over the graph. Supports: MATCH, WHERE, RETURN, LIMIT, ORDER BY.

**Arguments:**
- `query` (required): the query string

---

## Context tool

### get_task_context

Focused subgraph relevant to a described task, ranked by evidence-fusion scoring.

**Arguments:**
- `task_description` (required): natural language description of what you're working on
- `token_budget` (required): maximum tokens to include in the response (100–100000)
- `include_code` (optional): include source code snippets for top symbols
- `scope` (optional): file path or directory prefix to constrain search

**Ranking formula:**
```
score = (lexical_match × 0.30)
      + (embedding_similarity × 0.25)   ← 0.0 if embeddings unavailable
      + (scip_reference_distance × 0.20)
      + (git_recency × 0.15)
      + (edge_confidence × 0.10)
      + (file_size_penalty × −0.05)
```
When embeddings are unavailable, `lexical_match` weight becomes 0.55.

**Response includes:** `symbols`, `relationships`, `truncated`, `confidence` (0.0–1.0), `coverage_percent`, `reasons` (one-line reason per included file), `embeddings_used`.

**Guarantee:** never returns empty on a healthy index — falls back to file-proximity heuristic when graph signals are weak.

---

## Search tools

### search_text

Full-text search using FTS5 BM25 ranking.

**Arguments:**
- `query` (required): search terms
- `limit` (optional): max results, default 20

### semantic_search

Vector similarity search using local ONNX embeddings. Only available when `cortex semantic enable` has been run.

**Arguments:**
- `query` (required): natural language query
- `top_k` (optional): max results, default 10

---

## HTTP tools

### get_http_routes

All detected REST/GraphQL route definitions.

**Arguments:**
- `method` (optional): filter by HTTP method
- `path_prefix` (optional): filter by path prefix

### trace_http_call

Trace a cross-service HTTP call to its handler.

**Arguments:**
- `url_pattern` (required): URL pattern to trace

---

## Security tools

### find_taint_paths

Data flow paths from user input sources to sensitive sinks.

**Arguments:**
- `source_kind` (optional): filter by source kind (HttpInput, FileInput, EnvVar, UserSession)
- `sink_kind` (optional): filter by sink kind (SqlQuery, CommandExecution, FileWrite, HttpResponse, LogOutput)

### scan_owasp

OWASP Top 10 pattern detection.

**Arguments:**
- `category` (optional): filter by OWASP category (A01–A10)

### generate_sbom

SPDX 2.3 SBOM from the import graph.

**Arguments:**
- `format` (optional): output format, default "spdx"

### check_dependencies

Cross-reference SBOM against OSV.dev for known vulnerabilities.

**Arguments:**
- `repo_root` (optional): repository root path

---

## Memory tools

### write_observation

Store a note linked to a code symbol. Persists across sessions.

**Arguments:**
- `node_fqn` (required): the symbol this observation is about
- `observation_text` (required): the note content
- `agent_id` (optional): identifier for the agent writing the observation

### read_observations

Retrieve observations for a symbol.

**Arguments:**
- `fqn` (required): target symbol
- `include_stale` (optional): whether to include stale observations, default false

**Staleness:** when the linked node's content hash changes, observations are marked `is_stale: true`.

### write_adr

Store an architectural decision record.

**Arguments:**
- `title` (required): ADR title
- `body` (required): decision content
- `status` (optional): "proposed", "accepted", or "deprecated" (default "proposed")
- `linked_fqn` (optional): link to a specific code symbol

### read_adrs

Retrieve ADRs.

**Arguments:**
- `fqn` (optional): filter by linked symbol
- `status` (optional): filter by status

### prune_observations

Remove stale observations.

**Arguments:**
- `older_than_days` (optional): only prune observations older than N days

---

## Analysis tools

### decompose_boundaries

Leiden community detection on the call graph.

**Arguments:**
- `module_path` (optional): scope to a specific module
- `coupling_threshold` (optional): minimum coupling score, default 0.5

### get_complexity_hotspots

Functions ranked by cyclomatic complexity.

**Arguments:**
- `limit` (optional): max results, default 20
- `threshold` (optional): minimum complexity threshold, default 5

### generate_steering

Generate CLAUDE.md/AGENTS.md content from graph analysis. Includes module boundaries, complexity hotspots, and active ADRs. Output kept under 2000 tokens.

**Arguments:** none

### get_class_hierarchy

Query inheritance and interface implementation edges for a class.

**Arguments:**
- `fqn` (required): class or interface FQN
- `direction` (optional): "both" (default), "up" (parents only), or "down" (children only)

### get_git_hotspots

High-churn files ranked by risk score (git commit frequency × caller count).

**Arguments:**
- `limit` (optional): max results, default 20
- `since_months` (optional): how far back in git history, default 6

### get_import_graph

All import relationships for a file or module.

**Arguments:**
- `file` (optional): specific file path
- `module` (optional): module name prefix

### find_similar_functions

Find functions with similar call patterns (overlapping callee sets).

**Arguments:**
- `fqn` (required): target function
- `limit` (optional): max results, default 5
