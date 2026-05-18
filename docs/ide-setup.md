---
title: "IDE Setup"
description: "Configure your AI coding agent to use Cortex as an MCP server."
order: 2
category: "guides"
lastModified: "2025-01-15"
---

# IDE setup

The fastest way to configure your IDE is `cortex install`, which auto-detects installed agents and writes the correct config. The manual instructions below are for when you need more control.

In all examples, replace `/path/to/cortex` with the actual path to your cortex binary.

## Cursor

File: `.cursor/mcp.json` in your project root.

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

## Claude Desktop

File: `~/Library/Application Support/Claude/claude_desktop_config.json` (macOS) or `%APPDATA%\Claude\claude_desktop_config.json` (Windows).

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

Set the `CORTEX_REPO_ROOT` environment variable if you want Cortex to index a specific directory:

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"],
      "env": {
        "CORTEX_REPO_ROOT": "/path/to/your/project"
      }
    }
  }
}
```

## Kiro

File: `.kiro/settings/mcp.json` in your project root.

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

## Claude Code CLI

One command:

```sh
claude mcp add cortex /path/to/cortex serve
```

## Cline (VS Code extension)

File: `.vscode/mcp.json` in your project root.

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

Cline reads this file automatically when the extension loads.

## Windsurf

File: `.windsurf/mcp.json` in your project root.

```json
{
  "mcpServers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

## Zed

File: `.zed/settings.json` in your project root (or `~/.config/zed/settings.json` for global config).

```json
{
  "context_servers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

Note: Zed uses `context_servers` as the key, not `mcpServers`.

## VS Code Copilot Chat

VS Code Copilot Chat does not support MCP tool calls yet. Use Cline or another VS Code extension that supports MCP if you need Cortex in VS Code.

## Other agents

Cortex also supports auto-configuration for Aider, Continue.dev, Codex CLI, JetBrains IDEs, Supermaven, Codeium, and Tabnine. Run `cortex install` to configure them automatically.

## Environment variables

When running as an MCP server, Cortex determines the repository root from `CORTEX_REPO_ROOT`. If this is not set, it uses the current working directory. Most IDE integrations set the working directory to the project root automatically, so you usually do not need to set this.
