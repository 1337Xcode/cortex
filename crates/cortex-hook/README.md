# cortex-hook

What runs when an agent calls `cortex hook`, and the one client of the daemon's protocol, which the
CLI uses as well.

## What it must never do

- Depend on a Cortex crate other than `cortex-core`: a hook runs on every tool call and must start
  without loading the index, the store or a runtime.
- Wait for the daemon past its deadline, or block, deny or change an agent's tool call.

## Entry points

| Item | What it is |
| --- | --- |
| `ask` | Connects to a repository's daemon (its Unix socket, or its named pipe on Windows), sends the handshake and one request in a single write, and returns the daemon's handshake and answer, all before a deadline. The exchange runs on a thread of its own, since a named pipe takes no timeout; past the deadline the caller gets `TimedOut` and the thread is left to the process's exit |
| `ClientError` | `NotRunning` (nothing listens), `TimedOut`, `Mismatch` (the daemon speaks another protocol, with its versions), `TooLarge` (the request does not fit a frame; refused before connecting), `NotTheDaemon` (on Windows, a pipe of the daemon's name held by a process other than the one the user's run folder names: it is told nothing), `Protocol` (an answer broke the protocol) and `Io` |

## Tests

```sh
cargo test -p cortex-hook
```

The client tests run a scripted daemon on a real socket or named pipe: an answer, a mismatch,
silence until the deadline, a closed connection, broken frames, and no daemon at all.
