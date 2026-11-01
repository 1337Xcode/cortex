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
| `run` | `cortex hook`: the arguments after `hook`, the payload's input and the home folder in; nothing out. When no daemon answers it starts one in the background (its own process group on Unix; detached, in a process group of its own and outside the agent's job when allowed on Windows) and does not wait for it. A probe of `cortex doctor`'s starts none and asks no older daemon to stop: it measures the daemon that runs, and one that finds none is a line in the hook log. Every failure and panic ends in silence and a line in the hook log, by the event's hard timeout |
| `ask` | Connects to a repository's daemon (its Unix socket, or its named pipe on Windows), sends the handshake and one request in a single write, and returns the daemon's handshake and answer, all before a deadline. The exchange runs on a thread of its own, since a named pipe takes no timeout; past the deadline the caller gets `TimedOut` and the thread is left to the process's exit |
| `stop_if_older`, `stop_any`, `Stop` | Reads a daemon's handshake and, on that one connection, asks it to stop only when it is older than this build; a newer daemon, or one that replaced the older meanwhile, is left running. `stop_any` asks it to stop whatever its version, for a command that takes Cortex out of a repository |
| `start_daemon`, `start_daemon_now`, `Started` | Starts `cortex daemon --root <root>` from a given binary, detached, in the root, with no standard streams, unless a start for the repository was made in the last 10 s; returns at once. The hook starts one when it finds none. `start_daemon_now` starts one whatever the gap, for a command the user runs that finds none (init, or one replacing an older daemon), and marks the start so that hooks keep their gap |
| `ClientError` | `NotRunning` (nothing listens), `TimedOut`, `Mismatch` (the daemon speaks another protocol, with its versions), `TooLarge` (the request does not fit a frame; refused before connecting), `NotTheDaemon` (on Windows, a pipe of the daemon's name held by a process other than the one the user's run folder names: it is told nothing), `Protocol` (an answer broke the protocol) and `Io` |
| `locate` | The repository a folder belongs to: the nearest folder at or above it that holds `.cortex`, never the home folder or one above it; its canonical root and the root as the caller spelled it. The hook finds a payload's repository with it, and the CLI a command's |
| `adapter` | The adapter for the agent named on the command line (`claude`), or `None` |
| `Adapter` | What an agent's adapter knows: the agent, the hooks Cortex registers with it (`hooks`), the versions of the agent Cortex knows (`versions`, a `Versions`: the oldest its hooks work with and the newest its tests hold), and `normalize`, which turns one hook payload into a normalized event, or into no event when Cortex does not record that hook or the working folder is in no repository it serves. A payload marked `"cortex_probe": true` makes an event marked as a probe, which a daemon answers and does not record |
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

The unit tests cover the payload's size cap and time limit, the repository lookup (links, the home
folder), the panic guard and the log's rate limit and rotation. The `cortex hook` command itself is
tested through the binary in `cortex-cli`'s `tests/hook.rs` and `tests/daemon_down.rs`.

The client tests run a scripted daemon on a real socket or named pipe: an answer, a mismatch,
silence until the deadline, a closed connection, broken frames, and no daemon at all.

The Claude Code tests normalize every captured payload under `tests/fixtures/claude/` and compare
it with an event written out by hand from the fixture: paths, lines, commands, sizes and exit
codes. Property tests feed payloads of any shape, and captured payloads with a byte changed, and
check that nothing panics and that each event keeps to its hook's fields.
