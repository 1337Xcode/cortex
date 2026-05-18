---
title: "Architecture"
description: "Technical overview of Cortex internals, the call graph, and module structure."
order: 6
category: "concepts"
lastModified: "2025-01-15"
---

# Architecture

Cortex is a single Rust binary that runs three subsystems in one process.

## Overview

```mermaid
graph TD
    subgraph "cortex serve"
        Indexer[Indexer<br/>Rayon + tree-sitter]
        Watcher[File Watcher<br/>notify crate]
        MCP[MCP Server<br/>Tokio + JSON-RPC 2.0]
    end

    DB[(SQLite<br/>WAL mode)]

    Watcher -->|FileEvent| Indexer
    Indexer -->|Write| DB
    MCP -->|Read| DB
```

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant MCP as MCP Server
    participant Store as SQLite Store
    participant Indexer as Indexer

    Note over Indexer: Background: file watcher triggers re-index

    Agent->>MCP: tools/call (trace_callers, fqn="process_order")
    MCP->>Store: graph::trace_callers(fqn, depth=3)
    Store-->>MCP: Vec<CallPath> (BFS result)
    MCP-->>Agent: JSON response + _meta {tokens_used, tokens_saved}
```

## Indexer

The indexer walks the repository, parses each file with tree-sitter, and extracts:

- Symbols (functions, classes, methods, modules, interfaces, enums)
- Call edges (function A calls function B)
- Import relationships
- HTTP route definitions
- Taint sources and sinks

Parsing is parallelized with Rayon. Each file gets its own tree-sitter parser instance. Results are written to SQLite serially (single writer constraint).

```mermaid
flowchart TD
    File[Source File] --> Parse[tree-sitter Parse]
    Parse --> Extract[AST Extraction<br/>Nodes + Edges]
    Extract --> Security[Security Pass<br/>Taint sources/sinks]
    Extract --> Resolve[FQN Resolution<br/>Cross-file call edges]
    Security --> Delta[Delta Computation<br/>Compare to file_snapshots]
    Resolve --> Delta
    Delta --> Write[SQLite Write<br/>Single transaction]
    Write --> Invalidate[Memory Invalidation<br/>Mark stale observations]
```

On subsequent runs, the indexer skips files whose content hash has not changed. A full re-index of a medium project (100 files, 30K lines) takes about 500ms. Incremental re-index with no changes takes under 15ms.

## File watcher

When running in `serve` mode, Cortex starts a file watcher using the `notify` crate. It uses native OS file system events (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows).

When a file changes, the watcher triggers a re-index of just that file. The graph stays current without manual intervention.

## MCP server

The MCP server runs on a Tokio async runtime. It communicates over stdio using JSON-RPC 2.0, the standard MCP transport.

Each tool call is handled concurrently (up to 4 simultaneous calls by default). Read operations use a connection pool. Write operations go through a single writer connection.

```mermaid
flowchart LR
    subgraph "MCP Tools (32)"
        direction TB
        Structural[Structural<br/>search_symbols, trace_callers,<br/>trace_callees, get_file_context,<br/>get_architecture, find_dead_code,<br/>blast_radius, detect_changes,<br/>get_code_snippet, query_graph]
        Search[Search<br/>search_text, semantic_search]
        HTTP[HTTP<br/>get_http_routes, trace_http_call]
        Security[Security<br/>find_taint_paths, scan_owasp,<br/>generate_sbom, check_dependencies]
        Memory[Memory<br/>write_observation, read_observations,<br/>write_adr, read_adrs,<br/>prune_observations]
        Analysis[Analysis<br/>decompose_boundaries,<br/>get_complexity_hotspots,<br/>get_task_context,<br/>generate_steering,<br/>get_class_hierarchy,<br/>get_git_hotspots,<br/>get_import_graph,<br/>find_similar_functions]
    end
```

## Database schema

Cortex uses SQLite in WAL mode with a configurable read pool (1-16 connections, default 4).

```mermaid
erDiagram
    nodes {
        text fqn PK
        text kind
        text file
        int start_line
        int end_line
        text content_hash
    }
    edges {
        text caller_fqn FK
        text callee_fqn FK
        text kind
        int call_count
    }
    files {
        text path PK
        text content_hash
        int last_indexed
    }
    observations {
        int id PK
        text node_fqn FK
        text text
        text agent_id
        bool is_stale
        int created_at
    }
    adrs {
        int id PK
        text title
        text body
        text status
        text linked_fqn FK
    }

    nodes ||--o{ edges : "caller"
    nodes ||--o{ edges : "callee"
    nodes ||--o{ observations : "linked to"
    nodes ||--o{ adrs : "linked to"
```

Core tables:

- `nodes`: all extracted symbols (FQN, kind, file, line, column, content hash)
- `edges`: call relationships between nodes (caller FQN, callee FQN, kind, call count)
- `files`: tracked files with content hashes for change detection
- `observations`: agent memory linked to node FQNs with timestamps and staleness
- `adrs`: architectural decision records
- `fts_nodes`: FTS5 virtual table for full-text search over symbol names

Indexes exist on FQN, file path, and edge endpoints for fast lookups.

## Language support

Cortex uses tree-sitter grammars compiled into the binary. No external grammar files needed.

Supported languages (29, of which 26 use tree-sitter grammars and 3 use regex-based extraction: Kotlin, SQL, Perl):

| Language | Extensions |
|----------|-----------|
| Python | .py |
| TypeScript | .ts |
| TSX/JSX | .tsx, .jsx |
| JavaScript | .js, .jsx, .mjs |
| Go | .go |
| Rust | .rs |
| Java | .java |
| C# | .cs |
| C++ | .cpp, .cc, .cxx, .hpp, .h |
| C | .c |
| Ruby | .rb |
| Scala | .scala |
| Swift | .swift |
| PHP | .php |
| SQL | .sql |
| Kotlin | .kt, .kts |
| Dart | .dart |
| Elixir | .ex, .exs |
| Haskell | .hs |
| Lua | .lua |
| Zig | .zig |
| Bash/Shell | .sh, .bash |
| Perl | .pl, .pm |
| R | .r, .R |
| Objective-C | .m |
| OCaml | .ml, .mli |
| Julia | .jl |
| YAML | .yml, .yaml |
| Terraform/HCL | .tf, .hcl |

Each language has a tree-sitter query that extracts function definitions, class definitions, method definitions, and call expressions. Python, TypeScript, Rust, and Go have the most complete coverage.

## Security analysis

The security pass runs over the AST and call graph during indexing:

- Taint source/sink detection (HTTP inputs, SQL queries, file writes, command execution)
- Inter-procedural taint propagation via call graph edges
- OWASP Top 10 pattern matching against the structural graph
- SBOM generation from the import graph (SPDX format)
- Dependency vulnerability checking via OSV.dev

## Semantic search

When enabled (`cortex semantic enable`), Cortex downloads a local ONNX model (nomic-embed-text-v1, about 138 MB) and generates embeddings for all symbols. These are stored in the SQLite database via sqlite-vec and used for vector similarity search.

The model runs locally with no network calls after the initial download. Suitable for air-gapped environments.

## Bundle format

The `cortex bundle export` command produces a JSON file containing all nodes, edges, and observations. This file can be committed to the repository so teammates can query the graph without re-indexing.

```mermaid
flowchart LR
    DB[(SQLite<br/>graph.db<br/>gitignored)] -->|export| JSON[cortex.json<br/>committed to repo]
    JSON -->|import on checkout| DB2[(SQLite<br/>rebuilt from JSON)]
```

The bundle is JSON (not SQLite) because:
- JSON is diffable in pull requests
- Adding fields is backward-compatible
- Developers can open cortex.json and read the observations their team's agents wrote

## Memory layer

Agent observations are stored linked to specific code node FQNs. When the indexer detects that a node has changed (content hash differs), all observations linked to that node are marked stale.

Stale observations still surface in read results, but with a clear `is_stale: true` flag so the agent knows the note may be outdated.
