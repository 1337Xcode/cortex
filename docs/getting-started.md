---
title: "Getting Started"
description: "Install Cortex and index your first repository in under a minute."
order: 1
category: "guides"
lastModified: "2025-01-15"
---

# Getting started

## Install

The fastest way to install Cortex:

```sh
npx @1337xcode/cortex install
```

This downloads the binary for your platform, detects your AI coding agents, and writes the MCP config for each one. No manual configuration needed.

Alternatives:

```sh
# Shell script (macOS/Linux)
curl -fsSL https://raw.githubusercontent.com/1337Xcode/cortex/main/install.sh | sh

# Build from source
git clone https://github.com/1337Xcode/cortex.git && cd cortex
cargo build --release
cp target/release/cortex ~/.local/bin/
```

## Index your repository

```sh
cd /path/to/your/project
cortex index
```

This parses every source file with tree-sitter, extracts functions, classes, call edges, and import relationships, then stores them in a local SQLite database at `.cortex/graph.db`.

Indexing is fast. A 3500-file Python project (CPython) indexes in under 60 seconds. A typical 100-file project takes about 500ms.

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

If you didn't use `npx @1337xcode/cortex install`, configure manually:

```sh
cortex install
```

This detects installed agents and writes the correct config for each:

| Agent | Config location |
|-------|----------------|
| Claude Code | ~/.claude/settings.json |
| Claude Desktop (macOS) | ~/Library/Application Support/Claude/claude_desktop_config.json |
| Claude Desktop (Windows) | %APPDATA%\Claude\claude_desktop_config.json |
| Cursor | ~/.cursor/mcp.json |
| Windsurf | ~/.codeium/windsurf/mcp_config.json |
| VS Code | .vscode/mcp.json (workspace) |
| Zed | ~/.config/zed/settings.json |
| JetBrains | ~/.config/github-copilot/mcp.json |
| Kiro | ~/.kiro/settings/mcp.json |
| Cline | .cline/mcp.json |

## Verify it works

Ask your agent: "What are the most-called functions in this codebase?"

The agent should use the `get_architecture` or `search_symbols` tool and return structural results from the graph rather than reading files.

## What happens next

Once indexed, Cortex keeps the graph up to date automatically:

- The file watcher detects changes and re-indexes modified files in sub-second time
- Observations you write persist across sessions
- The bundle (cortex.json) can be committed to git for team sharing

## Common next steps

```sh
# Generate a security report
cortex security report

# See what breaks if you change a function
cortex impact MyClass.my_method

# Export an interactive graph visualization
cortex viz --export graph.html

# Set up auto re-indexing on commit
cortex hook install

# Add another repo to the federation
cortex federate add ../shared-lib

# Ingest documentation into the graph
cortex ingest ./docs

# Show build system workspace members
cortex modules --build-system

# Find high-churn risky code
cortex hotspots

# Cross-reference test coverage with call graph
cortex coverage --lcov coverage.lcov
```
