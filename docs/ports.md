---
title: "Network Ports"
description: "Network ports and transport protocols used by Cortex."
order: 7
category: "reference"
lastModified: "2025-01-15"
---

# Network Ports

Cortex uses minimal network resources. This page documents all transport mechanisms and ports so you can configure firewalls, proxies, or avoid conflicts with other services.

## MCP Server (stdio transport)

The MCP server communicates over **stdio** (stdin/stdout) using JSON-RPC 2.0. It does not open any network port.

When an AI agent launches Cortex (e.g., via `cortex serve`), the agent writes JSON-RPC requests to Cortex's stdin and reads responses from stdout. This is the standard MCP transport and requires no network configuration.

Because stdio is process-local, there is nothing to expose to the network, no firewall rules to add, and no port conflicts to worry about.

## Visualizer HTTP Server

The visualizer serves an interactive web UI (3D graph, dashboard, symbol explorer) over HTTP on **localhost**.

| Setting | Value |
|---------|-------|
| Default port | `9749` |
| Bind address | `127.0.0.1` (localhost only) |
| Protocol | HTTP |

### Enabling the visualizer

The visualizer is disabled by default. Enable it in one of two ways:

**Option 1: Standalone mode**

```sh
cortex viz
```

Opens the visualizer on port 9749 and launches your browser.

**Option 2: Alongside the MCP server**

Set `ui_enabled = true` in `.cortex/config.toml` (or `CORTEX_UI_ENABLED=true` as an environment variable), then run:

```sh
cortex serve
```

The visualizer starts on port 9749 in the background while the MCP server runs on stdio.

### Changing the port

In standalone mode, use the `--port` flag:

```sh
cortex viz --port 8080
```

When running via `cortex serve`, the visualizer currently uses port 9749. To use a different port in serve mode, set the `CORTEX_VIZ_PORT` environment variable:

```sh
CORTEX_VIZ_PORT=8080 cortex serve
```

Or add it to `.cortex/config.toml`:

```toml
viz_port = 8080
```

### Security note

The visualizer binds to `127.0.0.1` (localhost) by default. It is not accessible from other machines on the network. If you need remote access, place it behind a reverse proxy with appropriate authentication.

## Summary

| Component | Transport | Default Port | Configurable |
|-----------|-----------|--------------|--------------|
| MCP Server | stdio (stdin/stdout) | None (no network) | N/A |
| Visualizer | HTTP | 9749 | Yes (`--port` flag, `CORTEX_VIZ_PORT` env, or `viz_port` in config) |

No other network ports are opened by Cortex during normal operation.
