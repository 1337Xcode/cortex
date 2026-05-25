---
title: "MCP Tools"
description: "Documentation for all 32 MCP tools exposed by the Cortex server."
order: 4
category: "reference"
lastModified: "2025-07-14"
---

# MCP tools

Cortex exposes 32 tools over the Model Context Protocol. Every response includes `_meta` with token usage estimates.

## Meta-tool

### ask

Single-call code intelligence. Pass a natural language question and Cortex auto-routes to the appropriate internal tools, composing a unified answer. Agents no longer need to choose between 32 tools.

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

**Symbol extraction:** Automatically extracts symbol names from backtick-quoted identifiers, FQN paths (with `::`), snake_case, or CamelCase words in the question.

**Response includes:** `question`, `intent`, `symbol`, `results` (array of tool outputs), `_meta.tools_used`.

## Structural tools

### search_symbols

Find nodes by name pattern with optional kind filter. Includes coverage data when LCOV has been loaded.

**Arguments:**
- `pattern` (required): glob pattern to match against FQN (e.g., `*UserService*`, `src/auth*`)
- `kind` (optional): filter by node kind (Function, Class, Module, Route, Interface, Enum, Constant, Method)
- `limit` (optional): max results, default 50

**Behavior:** Searches the graph index first. If fewer than 3 results, automatically falls back to FTS5 BM25 search. Results are merged and deduplicated by FQN.

**Response includes:** `_meta.retrieval_method` ("graph", "fts5", or "hybrid")

### trace_callers

BFS over inbound call edges. Answers "who calls this function?"

**Arguments:**
- `fqn` (required): fully qualified name of the target node
- `depth` (optional): max traversal depth, default 3, max 5

**Returns:** array of CallPathNode with fqn, kind, file, start_line, depth, confidence, call_count. Sorted by call_count descending.

### trace_callees

BFS over outbound call edges. Answers "what does this function call?"

**Arguments:** same as trace_callers.

### get_file_context

Compressed structural summary of a file. Returns all symbols defined in it and their edges. Includes coverage data when LCOV has been loaded.

**Arguments:**
- `file` (required): relative file path

**Token savings:** approximately 500-800 tokens for a typical 300-line file vs 15,000+ tokens for the raw file content.

### get_architecture

High-level architecture summary. Languages, module structure, entry points, node/edge counts.

**Arguments:** none

**Token cost:** approximately 1000 tokens regardless of codebase size.

### find_dead_code

Nodes with zero inbound call edges, excluding entry points (route handlers, main functions, test functions, framework decorators).

**Arguments:**
- `limit` (optional): max results, default 50

### blast_radius

Given a node FQN, returns all nodes that transitively depend on it. Answers "what breaks if I change this?"

**Arguments:**
- `fqn` (required): target node
- `depth` (optional): max traversal depth, default 3

### detect_changes

Nodes modified since a given timestamp.

**Arguments:**
- `since` (required): Unix timestamp

**Returns:** nodes with change kind (added, modified, deleted) and risk score.

### get_code_snippet

Returns the actual source lines for a symbol.

**Arguments:**
- `fqn` (required): target node

**Behavior:** looks up the node's file and line range, reads from disk, returns the source. Detected secrets are redacted before returning.

### query_graph

Cypher-like query over the graph. Supports a defined subset: MATCH, WHERE, RETURN, LIMIT, ORDER BY.

**Arguments:**
- `query` (required): the query string

## Search tools

### search_text

Full-text search using FTS5 BM25 ranking.

**Arguments:**
- `query` (required): search terms
- `limit` (optional): max results, default 20

### semantic_search

Vector similarity search using local ONNX embeddings. Requires `cortex semantic enable` to have been run.

**Arguments:**
- `query` (required): natural language query
- `limit` (optional): max results, default 10

## HTTP tools

### get_http_routes

All detected REST/GraphQL route definitions.

**Arguments:**
- `method` (optional): filter by HTTP method
- `path_prefix` (optional): filter by path prefix

### trace_http_call

Trace a cross-service HTTP call to its handler.

**Arguments:**
- `method` (required): HTTP method
- `path` (required): route path

## Security tools

### find_taint_paths

Data flow paths from user input sources to sensitive sinks.

**Arguments:**
- `source_kind` (optional): filter by source kind (http_input, env_var, file_read)
- `sink_kind` (optional): filter by sink kind (sql_query, file_write, command_exec)

**Returns:** array of TaintPath with source_fqn, sink_fqn, path hops, confidence, CWE classification.

### scan_owasp

OWASP Top 10 pattern detection.

**Arguments:** none

**Returns:** array of findings with category (A01-A10), node_fqn, description, confidence.

### generate_sbom

SPDX 2.3 SBOM from the import graph.

**Arguments:**
- `repo_root` (optional): repository root path, default "."

### check_dependencies

Cross-reference SBOM against OSV.dev for known vulnerabilities.

**Arguments:**
- `repo_root` (optional): repository root path

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

**Staleness:** when the linked node's content hash changes (code was modified), observations are marked `is_stale: true`. They still appear but the agent should verify before trusting them.

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

## Analysis tools

### decompose_boundaries

Leiden community detection on the call graph.

**Arguments:**
- `module_path` (optional): scope to a specific module
- `coupling_threshold` (optional): minimum coupling score, default 0.5

**Returns:** communities with node_count, files, members, and inter-community coupling scores.

### get_complexity_hotspots

Functions ranked by complexity.

**Arguments:**
- `limit` (optional): max results
- `threshold` (optional): minimum complexity threshold

### get_task_context

Focused subgraph relevant to a described task.

**Arguments:**
- `task_description` (required): natural language description of what you're working on
- `token_budget` (required): maximum tokens to include in the response (integer, 100-100000)
- `include_code` (optional): include source code snippets for top symbols (boolean)
- `scope` (optional): file path or directory prefix to constrain search

### generate_steering

Generate CLAUDE.md/AGENTS.md content from graph analysis. Includes module boundaries from Leiden community detection, top complexity hotspots, and active architectural decision records.

**Arguments:** none

**Returns:** markdown content suitable for steering files, derived from module boundaries, complexity hotspots, and accepted ADRs. Output is kept under 2000 tokens; if exceeded, hotspots are truncated to top 5 and ADR summaries are reduced to title-only.

### get_class_hierarchy

Query inheritance and interface implementation edges for a class.

**Arguments:**
- `fqn` (required): class or interface FQN
- `direction` (optional): "both" (default), "up" (parents only), or "down" (children only)

**Returns:** parents (superclasses), children (subclasses), interfaces implemented, and implementors (if the target is an interface).

### get_git_hotspots

High-churn files ranked by risk score (git commit frequency combined with caller count).

**Arguments:**
- `limit` (optional): max results, default 20
- `since_months` (optional): how far back in git history, default 6

**Returns:** array of hotspot entries with file, fqn, churn_count, caller_count, and risk_score.

### get_import_graph

All import relationships for a file or module.

**Arguments:**
- `file` (optional): specific file path
- `module` (optional): module name prefix

**Returns:** array of import edges with source_fqn, target_fqn, and kind.

### find_similar_functions

Find functions with similar call patterns (overlapping callee sets).

**Arguments:**
- `fqn` (required): target function to find similar functions for
- `limit` (optional): max results, default 5

**Returns:** array of similar functions with fqn, similarity_score, and shared_callees count.
