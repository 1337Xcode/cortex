# cortex-mcp

Cortex's MCP server, which an agent starts as `cortex mcp` and talks to over stdio, built on
rmcp, the official Rust SDK. It offers one tool:

| Tool | Input | Output |
| --- | --- | --- |
| `cortex_where` | `query` (text), `limit` (1 to 5, 3 when left out) | The code the retrieval engine finds for the question, with its reasons and confidence, as `cortex where` prints it |

Every call is forwarded to the repository's daemon through the `Daemon` trait, which the
`cortex` binary implements: it finds the repository from the current folder at each call and
starts a daemon when none runs. An answer past 1,200 tokens is cut after its last whole line
that fits, with a note saying how many lines were left out. A call the daemon cannot answer is a
tool error the agent reads; the session goes on.

## What it must never do

- Print anything on standard output but the protocol.
- Send anything over the network, or read anything but the daemon's answers.
- Give a partial line, or more than 1,200 tokens in one answer.

## Entry points

- `serve(daemon)`: runs the server on stdio until the client ends the session.
- `Server::new(daemon)`: the server itself, for a transport of the caller's choice.
- `Daemon`: what the server asks; one method, `ask`.

## Tests

```sh
cargo test -p cortex-mcp
```

An MCP client from rmcp speaks to the server over an in-memory pipe, its daemon a fake that
records each request: the one tool and its schema, answers as the daemon wrote them, invalid
limits and empty questions refused, each failure a tool error, and the cap. The binary's tests
run `cortex mcp` against a real daemon in JSON-RPC lines and compare its answer with what
`cortex where` prints.
