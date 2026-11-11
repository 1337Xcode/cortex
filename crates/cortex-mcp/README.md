# cortex-mcp

Cortex's MCP server, which an agent starts as `cortex mcp` and talks to over stdio, built on
rmcp, the official Rust SDK. It offers three tools:

| Tool | Input | Output |
| --- | --- | --- |
| `cortex_where` | `query` (text), `limit` (1 to 5, 3 when left out) | The code the retrieval engine finds for the question, with its reasons and confidence, as `cortex where` prints it |
| `cortex_why` | `target` (a file path from the repository's root, `path:line`, `path:symbol`, or a symbol's name) | Why that code is where it is, as `cortex why` prints it: its file's strongest links by code, history and use with their strengths, the project facts about it true now, the sessions working on its file now and its open warnings; a target naming nothing or several things is a tool error in the daemon's words |
| `cortex_note` | `text` (the rule), `anchors` (file paths, `path:symbol`, or `.` for the whole repository, named from its root), `kind` (convention, decision, invariant, pitfall or ownership; convention when left out) | The rule kept as an agent's candidate fact, in the task of the session at work, and what became of it: a new candidate, a fact that says it already, or one that is now active |

Every call is forwarded to the repository's daemon through the `Daemon` trait, which the
`cortex` binary implements, with the local folders the client names as its roots. They are asked
once a session, and again after the client says they changed; a client that does not name them
within 2 s is not asked again in that session. The binary serves the repository `--root` names,
else the first root that is in a repository set up on this machine, else the current folder's,
and starts a daemon when none runs. Anywhere else, the home folder and a file system's root
included, the call is a tool error the agent can pass on: Cortex is not set up there (`cortex
init` sets it up), it is turned off there (`cortex on`), or the folder is never a repository. An
answer past 1,200 tokens is cut after its last whole line that fits, with a note saying how many
lines were left out. A call the daemon cannot answer is a
tool error the agent reads; the session goes on.

## What it must never do

- Print anything on standard output but the protocol.
- Send anything over the network, or read anything but the daemon's answers.
- Note an agent's rule as anything but a candidate.
- Give a partial line, or more than 1,200 tokens in one answer.

## Entry points

- `serve(daemon)`: runs the server on stdio until the client ends the session.
- `Server::new(daemon)`: the server itself, for a transport of the caller's choice.
- `Daemon`: what the server asks; one method, `ask`, given the request and the client's roots.

## Tests

```sh
cargo test -p cortex-mcp
```

An MCP client from rmcp speaks to the server over an in-memory pipe, its daemon a fake that
records each request and the roots given with it: the three tools and their schemas, a client's
roots given in its order (one that is no local folder left out), answers as the daemon wrote them, invalid
limits, empty questions, empty rules, missing anchors and unknown kinds refused, each failure a tool
error, and the cap. The binary's tests run `cortex mcp` against a real daemon in JSON-RPC lines,
compare its answer with what `cortex where` prints, and note a rule that `cortex facts list` then
shows as a candidate.
