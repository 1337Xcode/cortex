---
title: "Architecture"
description: "Technical overview of Cortex internals, the call graph, and module structure."
order: 6
category: "concepts"
lastModified: "2026-06-01"
---

# Architecture

Cortex is a single Rust binary that runs three subsystems in one process.

## Overview

```mermaid
graph TD
    subgraph "cortex serve"
        Indexer[Indexer<br/>Rayon + tree-sitter]
        SCIP[SCIP Ingester<br/>Protobuf → HIGH edges]
        FA[Framework Adapters<br/>FastAPI · Express · NestJS<br/>Spring · Django · React]
        Watcher[File Watcher<br/>notify crate]
        MCP[MCP Server<br/>Tokio + JSON-RPC 2.0]
        Viz[Visualizer<br/>3D Graph + Dashboard]
        HG[Health Gate]
    end

    DB[(SQLite<br/>WAL mode)]

    Watcher -->|FileEvent| Indexer
    Indexer -->|ast_direct 0.5| DB
    SCIP -->|scip 1.0| DB
    FA -->|framework_adapter 0.8| DB
    MCP -->|Read via Health Gate| HG
    HG -->|Healthy| DB
    HG -->|Unhealthy| Err[HealthError + FallbackSuggestion]
    Viz -->|Read| DB
```

```mermaid
sequenceDiagram
    participant Agent as AI Agent
    participant MCP as MCP Server
    participant HG as Health Gate
    participant Store as SQLite Store
    participant Indexer as Indexer

    Note over Indexer: Background: file watcher triggers re-index

    Agent->>MCP: tools/call (trace_callers, fqn="process_order")
    MCP->>HG: check_health()
    alt Index healthy
        HG-->>MCP: HealthStatus { healthy: true }
        MCP->>Store: graph::trace_callers_with_confidence(fqn, depth=3, min_confidence=0.7)
        Store-->>MCP: Vec<CallPathNode> with edge_source + confidence_tier
        MCP-->>Agent: JSON response + _meta {tokens_used, tokens_saved}
    else Index unhealthy
        HG-->>MCP: HealthStatus { healthy: false }
        MCP-->>Agent: HealthError { reason, suggested_action, fallback: FallbackSuggestion }
    end
```

## Indexer pipeline

The indexer runs in multiple passes per file:

1. **Framework detection** — scans dependency manifests to determine which adapters to activate
2. **tree-sitter parse** — extracts symbols and call edges (`edge_source=ast_direct`, `confidence=0.5`)
3. **SCIP ingestion** — additive pass that reads `.scip/index.scip` (or `index.scip`, `dump.lsif`) and creates HIGH-confidence edges (`edge_source=scip`, `confidence=1.0`); SCIP edges win over tree-sitter on conflicts
4. **Framework adapters** — pattern-match framework-specific wiring (DI, routing, middleware) and create MEDIUM-confidence edges (`edge_source=framework_adapter`, `confidence=0.8`)
5. **Pattern rules** — user-defined regex rules from `.cortex/patterns.toml`
6. **Embeddings** — incremental embedding generation (only re-embeds functions whose content hash changed)
7. **Health update** — writes `index_health` singleton row with file/node/edge counts and SCIP coverage

```mermaid
flowchart TD
    File[Source File] --> FD[Framework Detection<br/>scan manifests]
    FD --> Parse[tree-sitter Parse<br/>ast_direct · 0.5]
    Parse --> SCIP[SCIP Ingestion<br/>scip · 1.0<br/>dedup: SCIP wins]
    SCIP --> FA[Framework Adapters<br/>framework_adapter · 0.8]
    FA --> PR[Pattern Rules<br/>.cortex/patterns.toml]
    PR --> Security[Security Pass<br/>Taint + OWASP + SBOM]
    PR --> Resolve[FQN Resolution<br/>Cross-file call edges]
    Security --> Delta[Delta Computation<br/>Compare to file_snapshots]
    Resolve --> Delta
    Delta --> Write[SQLite Write<br/>Single transaction]
    Write --> Health[Update index_health]
    Write --> Invalidate[Memory Invalidation<br/>Mark stale observations]
```

On subsequent runs, the indexer skips files whose content hash has not changed. A full re-index of a medium project (100 files, 30K lines) takes about 500ms. Incremental re-index with no changes takes under 15ms.

## Confidence system

Every edge in the graph carries a `edge_source` and `confidence` value. Queries default to `confidence >= 0.7` (MEDIUM), filtering out heuristic name-match edges.

| Source | Confidence | Tier | How produced |
|--------|-----------|------|--------------|
| `scip` | 1.0 | HIGH | SCIP index (precise symbol resolution) |
| `framework_adapter` | 0.8 | MEDIUM | FastAPI/Express/NestJS/Spring/Django/React pattern matching |
| `ast_direct` | 0.5 | LOW | tree-sitter AST extraction |
| `name_match` | 0.3 | VERY_LOW | heuristic name-based resolution |

The `min_confidence` parameter on `trace_callers` and `blast_radius` lets agents lower or raise the threshold. All MCP tool responses include `edge_source` and `confidence_tier` per result.

## Framework adapters

Six adapters detect wiring that tree-sitter cannot see:

| Adapter | Patterns detected | Edge kinds |
|---------|------------------|-----------|
| FastAPI | `Depends(X)`, `@app.get/post`, `@router.*` | `Injects`, `Routes` |
| Express | `app.use(mw)`, `router.use(mw)`, `router.get/post/put/delete` | `Middleware`, `Routes` |
| NestJS | `@Controller()`, `@Injectable()` constructor injection | `Routes`, `Injects` |
| Spring | `@Autowired`, `@Inject`, `@Component/@Service/@Repository`, `@Bean` | `Injects` |
| Django | `urlpatterns path()/re_path()`, `@login_required` | `Routes`, `Middleware` |
| React | JSX component renders, `useContext(SomeContext)` | `Renders`, `Injects` |

Adapters only run for frameworks detected in dependency manifests. Manual override via `.cortex/config.toml` `frameworks = ["fastapi", "express"]`.

## Index health gate

All MCP tools check `index_health` before serving results. If `files_indexed == 0` or `node_count == 0` or `edge_count == 0`, every tool returns a `HealthError` with:
- `reason`: specific failure description
- `suggested_action`: e.g. "Run `cortex index`"
- `fallback`: `FallbackSuggestion` with grep commands and file-read suggestions

`cortex_status` and `get_repo_brief` are exempt from the health gate and always respond.

## Evidence-fusion ranking

`get_task_context` uses a multi-signal ranking formula:

```
score = (lexical_match × 0.30)
      + (embedding_similarity × 0.25)   ← 0.0 if embeddings unavailable
      + (scip_reference_distance × 0.20)
      + (git_recency × 0.15)
      + (edge_confidence × 0.10)
      + (file_size_penalty × −0.05)
```

When embeddings are unavailable, `lexical_match` weight becomes 0.55. Results are packed greedily into the token budget; top-1 is always included. Each result includes a one-line reason explaining why it was selected.

## File watcher

When running in `serve` mode, Cortex starts a file watcher using the `notify` crate. It uses native OS file system events (inotify on Linux, FSEvents on macOS, ReadDirectoryChangesW on Windows).

When a file changes, the watcher triggers a re-index of just that file. The graph stays current without manual intervention.

## MCP server

The MCP server runs on a Tokio async runtime. It communicates over stdio using JSON-RPC 2.0, the standard MCP transport.

Each tool call is handled concurrently (up to 4 simultaneous calls by default). Read operations use a connection pool. Write operations go through a single writer connection.

### Tool surface

Tools are classified into three tiers:

| Tier | Count | How to enable |
|------|-------|---------------|
| Default (always-on) | 10 | automatic |
| Experimental (opt-in) | 7 | `.cortex/config.toml` `experimental_tools = true` |
| Smart-tools mode | 5 | `cortex serve --smart-tools` |

`semantic_search` only appears in the manifest when embeddings are built.

```mermaid
flowchart LR
    subgraph "Default Tools (10)"
        direction TB
        D1[get_repo_brief]
        D2[get_task_context]
        D3[ask]
        D4[trace_callers]
        D5[blast_radius]
        D6[get_complexity_hotspots]
        D7[get_git_hotspots]
        D8[search_symbols]
        D9[write_observation]
        D10[read_observations]
    end
    subgraph "Experimental Tools (7)"
        direction TB
        E1[find_taint_paths]
        E2[check_dependencies]
        E3[decompose_boundaries]
        E4[generate_steering]
        E5[find_dead_code]
        E6[generate_sbom]
        E7[find_similar_functions]
    end
```

## Database schema

Cortex uses SQLite in WAL mode with a configurable read pool (1–16 connections, default 4).

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
        text source_fqn FK
        text target_fqn FK
        text kind
        real confidence
        text edge_source
    }
    file_snapshots {
        text file PK
        text content_hash
        int indexed_at
    }
    scip_coverage {
        text file PK
        int has_scip_data
        int symbols_resolved
        int indexed_at
    }
    index_health {
        int id PK
        int files_indexed
        int node_count
        int edge_count
        real scip_coverage_percent
        text frameworks_detected
        text health_status
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
    token_savings {
        int id PK
        text tool_name
        int tokens_used
        int baseline_cost
        int net_saved
        text query_terms
        int timestamp
    }
    repo_brief_cache {
        int id PK
        text brief_json
        int computed_at
        text index_hash
    }

    nodes ||--o{ edges : "source"
    nodes ||--o{ edges : "target"
    nodes ||--o{ observations : "linked to"
    nodes ||--o{ adrs : "linked to"
```

Core tables:

- `nodes`: all extracted symbols (FQN, kind, file, line, content hash)
- `edges`: relationships between nodes — now includes `edge_source` and `confidence`
- `file_snapshots`: tracked files with content hashes for change detection
- `scip_coverage`: per-file SCIP coverage tracking
- `index_health`: singleton row updated after every index run
- `observations`: agent memory linked to node FQNs with staleness tracking
- `adrs`: architectural decision records
- `token_savings`: per-query savings with honest `net_saved` (can be negative)
- `repo_brief_cache`: cached `get_repo_brief` output, invalidated on re-index
- `nodes_fts`: FTS5 virtual table for full-text search over symbol names

Schema migrations are numbered SQL files applied in order on startup (0001–0012).

## NodeKind classification

The indexer assigns `NodeKind::Method` to functions defined inside a class, struct, impl block, or trait implementation. Standalone functions get `NodeKind::Function`. The classification rules per language:

| Language | Method context |
|----------|---------------|
| Python | Function inside `class_definition` |
| TypeScript | Function inside `class_declaration` or `class_body` |
| Rust | Function inside `impl_item` or `trait_item` |
| Go | Function with receiver parameter |
| Java | Function inside `class_declaration` or `interface_declaration` |

Methods include the parent type name in their FQN: `file::ClassName::method_name`. Standalone functions use `file::function_name`.

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

## Security analysis

The security pass runs over the AST and call graph during indexing:

- Taint source/sink detection (HTTP inputs, SQL queries, file writes, command execution)
- Inter-procedural taint propagation via call graph edges
- OWASP Top 10 pattern matching against the structural graph
- SBOM generation from the import graph (SPDX format)
- Dependency vulnerability checking via OSV.dev

## Semantic search

When enabled (`cortex semantic enable`), Cortex downloads a local ONNX model (nomic-embed-text-v1, about 138 MB) or uses Ollama with `nomic-embed-code` if available. Embeddings are generated for all Function/Method/Class nodes and stored in the SQLite database via sqlite-vec.

Embedding generation is **incremental**: only nodes whose content hash changed since the last run are re-embedded. Stale embeddings for deleted nodes are removed automatically.

When embeddings are available, `get_task_context` uses cosine similarity as a 0.25-weight signal in evidence-fusion ranking. `cortex status` shows the embedding count and a degradation warning when running in BM25-only mode.

The `semantic_search` MCP tool only appears in the tool manifest when embeddings are built.

## Token savings accounting

After every query, Cortex records:

- `baseline_cost = (matching_file_count × avg_file_tokens) + grep_output_tokens`
- `net_saved = baseline_cost − (cortex_response_tokens + query_overhead_tokens)`

Negative values (Cortex cost more than grep would have) are stored and reported without modification. `cortex status --savings` shows the full dashboard.

## Bundle format

The `cortex bundle export` command produces a JSON file containing all nodes, edges, and observations. This file can be committed to the repository so teammates can query the graph without re-indexing.

```mermaid
flowchart LR
    DB[(SQLite<br/>graph.db<br/>gitignored)] -->|export| JSON[cortex.json<br/>committed to repo]
    JSON -->|import on checkout| DB2[(SQLite<br/>rebuilt from JSON)]
```

## Memory layer

Agent observations are stored linked to specific code node FQNs. When the indexer detects that a node has changed (content hash differs), all observations linked to that node are marked stale.

Stale observations still surface in read results, but with a clear `is_stale: true` flag so the agent knows the note may be outdated.

## Correctness benchmarks

`cortex benchmark` runs JSON-based test suites with ground-truth answers for `trace_callers`, `blast_radius`, `get_task_context`, and `ask`. The CI pipeline runs this on every release and fails the build if pass rate drops below 70%. `cortex status` displays a warning when the last benchmark run was below threshold.
