---
title: "Configuration"
description: "Configure Cortex behavior through cortex.toml, pricing.toml, and environment variables."
order: 5
category: "reference"
lastModified: "2025-07-14"
---

# Configuration

Cortex reads configuration from two sources, in priority order:

1. Environment variables (prefix: `CORTEX_`)
2. TOML file at `.cortex/config.toml` in the repository root

Environment variables always override file values.

## Environment variables

| Variable | Default | Description |
|----------|---------|-------------|
| `CORTEX_REPO_ROOT` | current directory | Root path of the repository to index |
| `CORTEX_DATA_DIR` | `.cortex/` | Directory for database and artifacts |
| `CORTEX_LOG_LEVEL` | `info` | Log level: trace, debug, info, warn, error |
| `CORTEX_MAX_TRAVERSAL_DEPTH` | `5` | Max depth for graph traversal (callers, callees, blast radius) |
| `CORTEX_MAX_GRAPH_QUERY_RESULTS` | `500` | Max results from a single graph query |
| `CORTEX_AUTO_INDEX` | `true` | Re-index automatically when files change |
| `CORTEX_UPDATE_CHECK` | `true` | Check for new versions on startup |
| `CORTEX_AUTO_BUNDLE_EXPORT` | `true` | Export bundle JSON after each index run |
| `CORTEX_ADDITIONAL_REPOS` | (empty) | Comma-separated paths for multi-repo mode |
| `CORTEX_UI_ENABLED` | `false` | Enable the visualizer HTTP server |
| `CORTEX_POOL_SIZE` | `4` | Number of read-only database connections (max 16) |

## Config file

Create `.cortex/config.toml` in your repository root:

```toml
repo_root = "."
data_dir = ".cortex"
log_level = "info"
max_traversal_depth = 5
max_graph_query_results = 500
auto_index = true
update_check = true
auto_bundle_export = true
pool_size = 4
additional_repos = []
```

All fields are optional. If a field is missing, the default value is used. If `repo_root` is omitted from both the environment and config file, Cortex uses the current working directory.

## Multi-repo mode

To index multiple repositories into a single graph, list additional paths:

```toml
additional_repos = ["/path/to/shared-lib", "/path/to/api-client"]
```

Or via environment variable:

```sh
export CORTEX_ADDITIONAL_REPOS="/path/to/shared-lib,/path/to/api-client"
```

## Pool size

The `pool_size` setting controls how many concurrent read connections the database uses. The default of 4 is fine for most projects. Increase it if you have many agents querying simultaneously. The maximum is 16. There is always exactly 1 write connection regardless of this setting.

## Data directory

By default, Cortex stores everything in `.cortex/` relative to the repository root. This includes:

- `graph.db` (SQLite database with WAL mode)
- `cortex.json` (exported bundle, if auto-export is enabled)
- `config.toml` (optional config file)
- Semantic search model files (if enabled)

Add `.cortex/` to your `.gitignore`. The `cortex.json` bundle file is the exception: you can commit it to share the graph with teammates who do not want to re-index locally.

## Model pricing

Cortex can report token cost estimates for different LLM providers. Configure pricing in `~/.cortex/pricing.toml`:

```toml
[[models]]
pattern = "claude-sonnet-4"
input_cost_per_million = 3.0
output_cost_per_million = 15.0

[[models]]
pattern = "gpt-5-4"
input_cost_per_million = 2.5
output_cost_per_million = 15.0

[[models]]
pattern = "gpt-4o"
input_cost_per_million = 2.5
output_cost_per_million = 10.0

[[models]]
pattern = "gemini-2-5-pro"
input_cost_per_million = 1.0
output_cost_per_million = 10.0
```

Each entry has three fields:

| Field | Type | Description |
|-------|------|-------------|
| `pattern` | string | Prefix pattern for model name matching |
| `input_cost_per_million` | float | USD cost per 1M input tokens |
| `output_cost_per_million` | float | USD cost per 1M output tokens |

When multiple patterns match a model name, Cortex uses the longest (most specific) prefix. For example, `gpt-4o-mini-2026` matches both `gpt-4o` and `gpt-4o-mini`, but the longer pattern wins.

If `~/.cortex/pricing.toml` does not exist, Cortex uses built-in defaults for common models. If the file contains invalid TOML, Cortex logs a warning and falls back to defaults.

## Ego-graph capping

When querying the ego-graph (subgraph centered on a node), Cortex caps results at 500 nodes to keep visualizations performant and responses compact.

Priority ordering when the cap is reached:

1. Nodes closer to the center (lower BFS depth) are included first
2. Within the same depth, nodes with more callers are prioritized

The response includes `truncated: true` and `total_reachable` count when the cap is applied, so you know how much of the graph was omitted.

## Update checking

When `update_check = true` (the default), Cortex checks for new versions on startup by comparing the local version against the latest GitHub Release tag. The check has a 3-second timeout and runs without blocking normal operation.

If a newer version is available, Cortex prints a notification to stderr with the update command:

```
Update available: v1.0.0 -> v1.0.3
Run: npx @1337xcode/cortex install
```

If the network request fails or times out, Cortex continues silently.
