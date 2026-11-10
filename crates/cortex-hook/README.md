# cortex-hook

What runs when an agent calls `cortex hook`, and the one client of the daemon's protocol, which the
CLI uses as well.

## What it must never do

- Depend on a Cortex crate other than `cortex-core`: a hook runs on every tool call and must start
  without loading the index, the store or a runtime.
- Wait for the daemon past its deadline, or block, deny or change an agent's tool call.
- Keep what a tool read, wrote or printed. A payload's file contents and edit text are skipped
  unread, a shell's output is measured as it is read, never kept, and a search's output is read
  for the files it names (at most 50 and 4 KiB of paths, inside the repository), never for the
  text it matched.
- Read more of an agent's transcript than the one value it looks for: for a Codex shell command,
  the exit code on the command's own line among the last mebibyte of the session's rollout
  file, a regular `rollout-*.jsonl` file under a Codex `sessions` folder.

## Entry points

| Item | What it is |
| --- | --- |
| `run` | `cortex hook`: the arguments after `hook`, the payload's input, the home folder and the output to write to the agent on. At a hook where Cortex gives the agent text (a session's start, a prompt that may ask for code, the moment before a read, a successful edit's result, a stop; a slash command, a shell escape or a prompt of fewer than four characters other than spaces goes as the event alone, and the daemon retrieves nothing for it) it asks the daemon for that text with the event (the primer, the work order, the read's hint, the edit's warnings, strict mode's reason to go on) and writes it on one line in the agent's hook protocol, once the whole answer came before the event's deadline, from its own thread after the work, so that no write is cut short; a write that fails is noted; anything else writes nothing. When no daemon answers it starts one in the background (its own process group on Unix; detached, in a process group of its own and outside the agent's job when allowed on Windows) and does not wait for it. A probe of `cortex doctor`'s starts none and asks no older daemon to stop: it measures the daemon that runs, and one that finds none is a line in the hook log. Every failure and panic ends in silence and a line in the hook log, by the event's hard timeout; a line that cannot be written by then, on a file system that stalls, is given up |
| `ask` | Connects to a repository's daemon (its Unix socket, or its named pipe on Windows), sends the handshake and one request in a single write, and returns the daemon's handshake and answer, all before a deadline. The exchange runs on a thread of its own, since a named pipe takes no timeout; past the deadline the caller gets `TimedOut` and the thread is left to the process's exit |
| `stop_if_older`, `stop_any`, `Stop` | Reads a daemon's handshake and, on that one connection, asks it to stop only when it is older than this build; a newer daemon, or one that replaced the older meanwhile, is left running. `stop_any` asks it to stop whatever its version, for a command that takes Cortex out of a repository |
| `start_daemon`, `start_daemon_now`, `Started` | Starts `cortex daemon --root <root>` from a given binary, detached, in the root, with no standard streams, unless a start for the repository was made in the last 10 s; returns at once. On Windows the process it starts is `cortex daemon --relaunch`, which starts the daemon anew with handle inheritance off and exits, so that the daemon holds no handle of the hook or of the agent that ran it: an agent that waits for a pipe it handed down, as Codex does at its exit, sees it end with the hook. The hook starts one when it finds none. `start_daemon_now` starts one whatever the gap, for a command the user runs that finds none (init, or one replacing an older daemon), and marks the start so that hooks keep their gap |
| `relaunch_daemon`, `RELAUNCH_FLAG` | Windows: starts `cortex daemon --root <root>` from a given binary through `CreateProcessW` with inheritance off, quoting the command line as Windows reads it, and returns without waiting; a failure is also written to the hook log of the home folder given, since the caller's own error output goes nowhere. `cortex daemon --relaunch` calls it |
| `ClientError` | `NotRunning` (nothing listens), `TimedOut`, `Mismatch` (the daemon speaks another protocol, with its versions), `TooLarge` (the request does not fit a frame; refused before connecting), `NotTheDaemon` (on Windows, a pipe of the daemon's name held by a process other than the one the user's run folder names: it is told nothing), `Protocol` (an answer broke the protocol) and `Io` |
| `locate` | The repository a folder belongs to: the nearest folder at or above it that holds `.cortex` (but not one holding the committed `shared` folder alone, as a clone of a repository whose team shares facts does, files such as `.DS_Store` aside); `locate_marked` finds any `.cortex`, as commands do, so that a fresh clone's root is still its root; never the home folder or one above it; its canonical root and the root as the caller spelled it. The hook finds a payload's repository with it, and the CLI a command's |
| `adapter` | The adapter for the agent named on the command line (`claude`, `codex`, `qwen`, `droid`, `devin`), or `None` |
| `Adapter` | What an agent's adapter knows: the agent, the hooks Cortex registers with it (`hooks`), the versions of the agent Cortex knows (`versions`, a `Versions`: the oldest its hooks work with and the newest its tests hold; none for an adapter built from the agent's documentation and not yet run live), `normalize`, which turns one hook payload into a normalized event, or into no event when Cortex does not record that hook or the working folder is in no repository it serves, and `context_output`, which writes text as the hook prints it for the agent to add to its context (for Claude Code, `{"hookSpecificOutput":{"hookEventName":...,"additionalContext":...}}`), or nothing when the agent would not take the text as it is (for Claude Code, past 10,000 characters, or holding a `system-reminder` tag, which it rewrites; for Codex, past 10,000 bytes). A payload marked `"cortex_probe": true` makes an event marked as a probe, which a daemon answers and does not record |
| `HookEntry` | One registered hook: the agent's event name, the tools it matches, the hook kind, its hard timeout, and whether Cortex gives the agent text there (for Claude Code, at `SessionStart`, `UserPromptSubmit`, `PreToolUse` on a read, for its hint, `PostToolUse`, `PostToolUseFailure`, for a failed check's map, and `Stop`, for strict mode's reason, which Claude Code takes as a reason to go on). A shell command that failed carries the end of what it printed (Claude Code's failure message after its `Exit code N` line; Codex's shell response when its rollout gives a non-zero exit code), at most `cortex_core::MAX_FAILED_OUTPUT_BYTES`, to the daemon's memory only |
| `Located`, `Stamp` | What `normalize` is given: where the working folder's repository is (its canonical root, and the root as the agent spells it), and the event's id and time |
| `NormalizeError` | `Json` (the payload is not the agent's JSON) and `NoSession` (it names no usable session) |

## Adding an agent

Write one module that implements `Adapter` and add one line to `ADAPTERS` in `src/adapter.rs`. An
agent whose hooks speak the JSON Claude Code defined uses `src/shaped/`: its module holds its
hooks table and a `Tools` reader (its tool names, what a call aimed at, how it ended), as
`src/claude/` and `src/codex/` do; an agent known only from its documentation fills a
`ToolTable` (`src/shaped/table.rs`: its tool names and kinds, the fields its payloads hold
elsewhere than Claude Code's as moves between JSON pointers, what its tool events say of a
call's end, where its prompt and working folder are), as `src/qwen.rs` and
`src/antigravity.rs` do. An agent whose hooks are a plugin's code (OpenCode, Kilo Code, Amp,
OpenClaw) also returns its plugin's files from `plugin_files`: the JavaScript core in
`plugins/core.js`, which runs `cortex hook` and never throws, then the agent's glue in
`plugins/<agent>.js`, which sends each event in Claude Code's JSON and puts the text it gets
back where the agent takes text.

## Tests

```sh
cargo test -p cortex-hook
```

The unit tests cover the payload's size cap, its time limit and the end of its JSON value, the
repository lookup (links, the home folder), the panic guard and the log's rate limit and
rotation. The `cortex hook` command itself is tested through the binary in `cortex-cli`'s
`tests/hook.rs`, `tests/answers.rs` (the primer and the work order as Claude Code gets them) and
`tests/daemon_down.rs`.

The client tests run a scripted daemon on a real socket or named pipe: an answer, a mismatch,
silence until the deadline, a closed connection, broken frames, and no daemon at all. The `run`
tests call the hook's entry point against the same scripted daemon and read what it writes and the
hook log: the text answered at a prompt and at a session's start is written as Claude Code's hook
JSON; no text or an empty one, an answer of another kind, a text Claude Code would not take as it
is, a probe's text, and a late or broken answer write nothing; a write the agent refuses is noted;
an answer that takes nothing is noted, for an event and for `cortex doctor`'s probe, and a note the
hook's own thread writes is in the log by the time the hook returns.

The Claude Code tests normalize every captured payload under `tests/fixtures/claude/` and compare
it with an event written out by hand from the fixture: paths, lines, commands, sizes, exit codes
and the files a search found. Grep's output is tested for each kind of line it prints (a match, a
count, context, a line without its number, a search of one file), in written cases and in captured
runs given a folder, one file, context lines and count mode, for files named twice, outside the
repository or past the first 200 named, and for its caps at 50 files and 4 KiB of paths.
Property tests feed payloads of any shape, captured payloads with a byte changed, and search
responses of any length and shape, and check that nothing panics, that each event keeps to its
hook's fields, and that a search's files stay within their caps.

The Codex tests normalize every payload under `tests/fixtures/codex/` against events written out
by hand, and test how a shell command is read: the programs that read a file or search, the
lines a read names (`head`, `sed -n`, `-TotalCount`, a read piped to them), the commands that
are only shell commands (a file's last lines, several files, a filter), a list's first command,
quotes; the files a patch's headers name and the exit code its response opens with; a shell's
and a read's exit code from its line in a rollout under a sessions folder (not from another
command's line, a rollout elsewhere, or past the last mebibyte); and any payload and any
command for panics.

The adapters built from documentation (Qwen Code, Droid, Devin, OpenHands, TRAE, Junie, Augment,
Gemini CLI, Cursor, Copilot, Antigravity, Kiro, Cline, Crush, Hermes Agent) normalize payloads typed from
each agent's documentation under `tests/fixtures/<agent>/docs-2026-11-06/` against events written
out by hand: an agent's own fields moved to Claude Code's places (Antigravity's tool call,
Cline's fields under each event's name, Hermes' `extra`), Qwen Code's own sends and Antigravity's
later model calls no prompts, a read with an offset or its lines is not recorded, results
follow what each documentation says its events mean; Devin's payloads, which name no folder,
make events only with `DEVIN_PROJECT_DIR` set, tested through the binary.

Each generated plugin is compared byte for byte with its golden copy under
`tests/fixtures/<agent>/plugin-2026-11-06/generated/`, and the calls those copies were recorded
making under Node (`node tests/plugins/record.mjs`, with a fake host and a fake `cortex`) are
normalized against events written out by hand; OpenClaw's, which name no folder, make events
only with `OPENCLAW_WORKSPACE_DIR` set, tested through the binary.
