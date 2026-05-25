---
title: "IDE Setup"
description: "Configure your AI coding agent to use Cortex as an MCP server."
order: 2
category: "guides"
lastModified: "2025-07-14"
---

# IDE setup

The fastest way to configure any agent is `cortex install`, which auto-detects installed agents and writes the correct config. Several agents also have dedicated subcommands for one-step setup.

In all manual examples, replace `/path/to/cortex` with the actual path to your cortex binary (`which cortex` on Unix, `where cortex` on Windows).

## Quick install reference

| Platform | Command |
|---|---|
| Claude Code (Linux/Mac) | `cortex install` |
| Claude Code (Windows) | `cortex install --platform claude-code` |
| Codex | `cortex install --platform codex` |
| OpenCode | `cortex install --platform opencode` |
| GitHub Copilot CLI | `cortex install --platform copilot` |
| VS Code Copilot Chat | `cortex vscode install` |
| Aider | `cortex install --platform aider` |
| OpenClaw | `cortex install --platform openclaw` |
| Factory Droid | `cortex install --platform droid` |
| Trae | `cortex install --platform trae` |
| Trae CN | `cortex install --platform trae-cn` |
| Gemini CLI | `cortex install --platform gemini` |
| Hermes | `cortex install --platform hermes` |
| Kimi Code | `cortex install --platform kimi` |
| Kiro IDE/CLI | `cortex kiro install` |
| Pi coding agent | `cortex install --platform pi` |
| Cursor | `cortex cursor install` |
| Google Antigravity | `cortex antigravity install` |

---

## Cursor

Dedicated subcommand (creates `.cursor/` if it does not exist):

```sh
cortex cursor install
```

Or manually, file: `.cursor/mcp.json` in your project root.

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

One command (Linux/Mac):

```sh
cortex install
```

Windows or explicit:

```sh
cortex install --platform claude-code
```

Or use the Claude Code CLI directly:

```sh
claude mcp add cortex /path/to/cortex serve
```

Manual config file: `~/.claude/settings.json`

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

## Kiro IDE/CLI

Dedicated subcommand:

```sh
cortex kiro install
```

Manual config file: `.kiro/settings/mcp.json` in your project root.

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

## Google Antigravity

Dedicated subcommand (writes to `~/.antigravity/mcp.json`):

```sh
cortex antigravity install
```

Or:

```sh
cortex install --platform antigravity
```

## Codex CLI

```sh
cortex install --platform codex
```

Manual config file: `~/.codex/mcp.json`

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

## OpenCode

```sh
cortex install --platform opencode
```

Manual config file: `~/.opencode/mcp.json`

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

## GitHub Copilot CLI

```sh
cortex install --platform copilot
```

Manual config file: `.github/copilot-mcp.json` in your project root.

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

## VS Code Copilot Chat

Dedicated subcommand:

```sh
cortex vscode install
```

Or with `--platform`:

```sh
cortex install --platform vscode
```

Manual config file: `.vscode/mcp.json` in your project root.

```json
{
  "servers": {
    "cortex": {
      "command": "/path/to/cortex",
      "args": ["serve"]
    }
  }
}
```

Note: VS Code uses `servers` as the root key, not `mcpServers`.

## Aider

```sh
cortex install --platform aider
```

Creates `.aider.mcp.json` in your project root (or `~/.aider.mcp.json` if no project config exists).

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

## OpenClaw

```sh
cortex install --platform openclaw
```

Manual config file: `~/.openclaw/mcp.json`

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

## Factory Droid

```sh
cortex install --platform droid
```

Manual config file: `~/.droid/mcp.json`

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

## Trae

```sh
cortex install --platform trae
```

Manual config file: `~/.trae/mcp.json`

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

## Trae CN

```sh
cortex install --platform trae-cn
```

Manual config file: `~/.trae-cn/mcp.json`

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

## Gemini CLI

```sh
cortex install --platform gemini
```

Manual config file: `~/.gemini/settings.json`

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

## Hermes

```sh
cortex install --platform hermes
```

Manual config file: `~/.hermes/mcp.json`

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

## Kimi Code

```sh
cortex install --platform kimi
```

Manual config file: `~/.kimi/mcp.json`

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

## Pi coding agent

```sh
cortex install --platform pi
```

Manual config file: `~/.pi/mcp.json`

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

## Windsurf

```sh
cortex install --platform windsurf
```

Manual config file: `.windsurf/mcp.json` in your project root.

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

```sh
cortex install --platform zed
```

Manual config file: `.zed/settings.json` in your project root (or `~/.config/zed/settings.json` for global config).

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

## Cline (VS Code extension)

```sh
cortex install --platform cline
```

Manual config file: `.vscode/mcp.json` in your project root.

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
      "args": ["serve"],
      "env": {
        "CORTEX_REPO_ROOT": "/path/to/your/project"
      }
    }
  }
}
```

## Other agents

Cortex also supports Continue.dev, JetBrains, Supermaven, Codeium, and Tabnine. Run `cortex install` to auto-configure all detected agents at once.

## Environment variables

When running as an MCP server, Cortex determines the repository root from `CORTEX_REPO_ROOT`. If this is not set, it uses the current working directory. Most IDE integrations set the working directory to the project root automatically, so you usually do not need to set this.
