---
title: "Configuration"
description: "Configure Cortex behavior through cortex.toml and environment variables."
order: 5
category: "reference"
lastModified: "2025-01-15"
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
