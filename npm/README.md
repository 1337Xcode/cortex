# @1337xcode/cortex

Code intelligence for AI agents. Indexes your repo into a call graph, exposes it over MCP. Agents query the graph instead of reading files.

[![npm](https://img.shields.io/npm/v/@1337xcode/cortex)](https://www.npmjs.com/package/@1337xcode/cortex)
[![MIT](https://img.shields.io/badge/license-MIT-green)](https://github.com/1337Xcode/cortex/blob/main/LICENSE)

## Install

```sh
npx @1337xcode/cortex install
```

Downloads the binary for your platform, verifies the SHA256 checksum, drops it in your PATH. Works on Linux, macOS (Intel + Apple Silicon), and Windows.

## What it does

Your AI agent asks "what calls processOrder?" and gets a 200-token graph result instead of burning 20,000 tokens reading files. That's the pitch.

Cortex parses your code with tree-sitter (29 languages), extracts functions/classes/imports/call edges, stores everything in SQLite. Agents connect via MCP and query the graph.

## Quick start

```sh
cortex index     # parse repo, build the graph
cortex install   # detect your IDE, write MCP config
cortex serve     # start the MCP server
```

## Numbers

- 29 languages via tree-sitter
- 32 MCP tools (or 5 in smart mode via the `ask` meta-tool)
- 127 files indexed in 535ms, incremental re-index in 13ms
- 10-100x token reduction on structural questions

## What else

- Taint flow analysis, OWASP Top 10 detection, SBOM generation
- Cross-session memory that marks observations stale when code changes
- Multi-repo federation for querying across repositories
- 3D interactive graph visualization (`cortex viz --export graph.html`)
- CI quality gates with configurable thresholds
- Git hotspot analysis, coverage gap detection
- Works offline. No cloud, no API keys, no Docker.

## MCP config

```json
{
  "mcpServers": {
    "cortex": {
      "command": "npx",
      "args": ["@1337xcode/cortex", "serve"]
    }
  }
}
```

## Security

Single binary. No network access. No telemetry. No env var harvesting. SHA256 verified on install. Full source at [github.com/1337Xcode/cortex](https://github.com/1337Xcode/cortex).

## Docs

[1337xcode.github.io/cortex](https://1337xcode.github.io/cortex)

## License

MIT
