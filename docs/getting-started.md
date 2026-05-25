---
title: "Getting Started"
description: "Install Cortex and index your first repository in under a minute."
order: 1
category: "guides"
lastModified: "2026-05-25"
---

# Getting started

## Install

The fastest way to install Cortex:

```sh
npx @1337xcode/cortex@latest install
```

This downloads the binary for your platform, verifies the SHA-256 checksum, removes any stale old versions, configures your PATH, and runs a full index. Zero-touch.

Alternatives:

```sh
# Shell script (macOS/Linux)
curl -fsSL https://raw.githubusercontent.com/1337Xcode/cortex/main/install.sh | sh

# Build from source
git clone https://github.com/1337Xcode/cortex.git && cd cortex
cargo build --release
cp target/release/cortex ~/.local/bin/
```

## Update

Cortex checks for updates on every launch and notifies you if a newer version exists. To update:

```sh
cortex update
```

This downloads the latest release, verifies the checksum, replaces the binary, and re-indexes.

## Index your repository

```sh
cd /path/to/your/project
cortex index
```

This parses every source file with tree-sitter (29 languages), extracts functions, classes, call edges, and import relationships, then stores them in a local SQLite database.

Indexing is fast. A 3500-file Python project (CPython) indexes in under 60 seconds. A typical 100-file project takes about 500ms.

To force a clean rebuild:

```sh
cortex reindex
```

## Start the MCP server

```sh
cortex serve
```

This starts the MCP server over stdio (JSON-RPC 2.0). Your AI agent connects to it and gains access to 32 structural query tools.

For reduced context overhead (agents see 5 tools instead of 32):

```sh
cortex serve --smart-tools
```

In smart-tools mode, the `ask` meta-tool handles everything. The agent sends a natural language question, Cortex routes internally to the right graph queries, and returns a composed answer.

You don't usually run this manually. The `cortex install` command configures your agent to launch `cortex serve` automatically when it starts a session.

## Configure your agent

If you didn't use `npx @1337xcode/cortex@latest install`, configure manually:

```sh
cortex install
```

This detects installed agents and writes the correct config for each:

| Agent | Config location |
|-------|----------------|
| Claude Code | ~/.claude/settings.json |
| Cursor | .cursor/mcp.json |
| Windsurf | .windsurf/mcp.json |
| VS Code | .vscode/mcp.json |
| Kiro | .kiro/settings/mcp.json |
| Zed | .zed/settings.json |
| JetBrains | .idea/mcp.json |
| GitHub Copilot | .github/copilot-mcp.json |
| Cline/Roo | .vscode/mcp.json |

Use `--platform <name>` to configure a specific agent without auto-detection.

## Verify it works

Ask your agent: "What are the most-called functions in this codebase?"

The agent should use the `get_architecture` or `search_symbols` tool and return structural results from the graph rather than reading files.

## Visualize

```sh
# Interactive 3D graph in browser (live server)
cortex serve
# Then open http://127.0.0.1:9749

# Export standalone HTML file
cortex viz --export graph.html
```

## Uninstall

To completely remove Cortex (binary, database, config, PATH entries):

```sh
cortex uninstall
```

## Common next steps

```sh
cortex security report     # Security scan
cortex impact MyClass.foo  # Blast radius analysis
cortex viz --export g.html # Export 3D graph
cortex hook install        # Auto re-index on commit
cortex federate add ../lib # Multi-repo federation
cortex ingest ./docs       # Ingest documentation
cortex hotspots            # Find risky code
cortex coverage --lcov c.lcov  # Coverage gaps
```
