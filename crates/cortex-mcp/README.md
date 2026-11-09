# cortex-mcp

Cortex's MCP server, which an agent starts as `cortex mcp` and talks to over stdio, built on
rmcp, the official Rust SDK. It offers two tools:

| Tool | Input | Output |
| --- | --- | --- |
| `cortex_where` | `query` (text), `limit` (1 to 5, 3 when left out) | The code the retrieval engine finds for the question, with its reasons and confidence, as `cortex where` prints it |
| `cortex_note` | `text` (the rule), `anchors` (file paths, `path:symbol`, or `.` for the whole repository, named from its root), `kind` (convention, decision, invariant, pitfall or ownership; convention when left out) | The rule kept as an agent's candidate fact, in the task of the session at work, and what became of it: a new candidate, a fact that says it already, or one that is now active |

Every call is forwarded to the repository's daemon through the `Daemon` trait, which the
`cortex` binary implements: it finds the repository from the current folder at each call and
starts a daemon when none runs. An answer past 1,200 tokens is cut after its last whole line
that fits, with a note saying how many lines were left out. A call the daemon cannot answer is a
tool error the agent reads; the session goes on.

## What it must never do

- Print anything on standard output but the protocol.
- Send anything over the network, or read anything but the daemon's answers.
- Note an agent's rule as anything but a candidate.
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
records each request: both tools and their schemas, answers as the daemon wrote them, invalid
limits, empty questions, empty rules, missing anchors and unknown kinds refused, each failure a tool
error, and the cap. The binary's tests run `cortex mcp` against a real daemon in JSON-RPC lines,
compare its answer with what `cortex where` prints, and note a rule that `cortex facts list` then
shows as a candidate.
