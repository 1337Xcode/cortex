# cortex-hook

What runs when an agent calls `cortex hook`, and the one client of the daemon's protocol, which the
CLI uses as well.

## What it must never do

- Depend on a Cortex crate other than `cortex-core`: a hook runs on every tool call and must start
  without loading the index, the store or a runtime.
- Wait for the daemon past its deadline, or block, deny or change an agent's tool call.
- Keep what a tool read, wrote or printed. A payload's file contents and edit text are skipped
  unread, and a shell's output is measured as it is read, never kept.

## Entry points

| Item | What it is |
| --- | --- |
| `ask` | Connects to a repository's daemon (its Unix socket, or its named pipe on Windows), sends the handshake and one request in a single write, and returns the daemon's handshake and answer, all before a deadline. The exchange runs on a thread of its own, since a named pipe takes no timeout; past the deadline the caller gets `TimedOut` and the thread is left to the process's exit |
| `ClientError` | `NotRunning` (nothing listens), `TimedOut`, `Mismatch` (the daemon speaks another protocol, with its versions), `TooLarge` (the request does not fit a frame; refused before connecting), `NotTheDaemon` (on Windows, a pipe of the daemon's name held by a process other than the one the user's run folder names: it is told nothing), `Protocol` (an answer broke the protocol) and `Io` |
| `adapter` | The adapter for the agent named on the command line (`claude`), or `None` |
| `Adapter` | What an agent's adapter knows: the agent, the hooks Cortex registers with it (`hooks`), and `normalize`, which turns one hook payload into a normalized event, or into no event when Cortex does not record that hook or the working folder is in no repository it serves |
| `HookEntry` | One registered hook: the agent's event name, the tools it matches, the hook kind and its hard timeout |
| `Located`, `Stamp` | What `normalize` is given: where the working folder's repository is (its canonical root, and the root as the agent spells it), and the event's id and time |
| `NormalizeError` | `Json` (the payload is not the agent's JSON) and `NoSession` (it names no usable session) |

## Adding an agent

Write one module that implements `Adapter` (see `src/claude/`) and add one line to `ADAPTERS` in
`src/adapter.rs`.

## Tests

```sh
cargo test -p cortex-hook
```

The client tests run a scripted daemon on a real socket or named pipe: an answer, a mismatch,
silence until the deadline, a closed connection, broken frames, and no daemon at all.

The Claude Code tests normalize every captured payload under `tests/fixtures/claude/` and compare
it with an event written out by hand from the fixture: paths, lines, commands, sizes and exit
codes. Property tests feed payloads of any shape, and captured payloads with a byte changed, and
check that nothing panics and that each event keeps to its hook's fields.
