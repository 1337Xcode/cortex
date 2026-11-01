# cortex-daemon

The process that keeps one repository's index: the store's only writer, the file watcher, and in
huge mode the crawler that parses files in the background. `cortex daemon` runs it.

## What it must never do

- Let anything but its writer write the store, or run twice for one repository: the run folder's
  lock refuses a second daemon, and the store's writer lock refuses a second writer.
- Use the network. It asks git for the user's global gitignore with `git config` alone, which reads
  configuration files and nothing else.
- Read the repository's own git configuration for that: a repository could then name the file the
  daemon opens.
- Write code, prompt text or tool output to its log. A line holds the daemon's own words, ids,
  counts, times and paths, at most 1,000 characters on one line.

## Entry points

| Item | What it is |
| --- | --- |
| `run` | Takes the repository's hold first (a lock on `~/.cortex/run/<id>.lock`, released by the system when the process ends, and `<id>.pid` naming the process; a second daemon is refused, naming the first's process), then runs the daemon for a repository's canonical root until it has been idle for `daemon.idle_shutdown_minutes` (no request of any kind, no file change) with nothing waiting to be parsed or recorded; configuration warnings and the ready line go to a callback |
| (endpoint) | Bound right after the run lock, before the store opens: on Unix `~/.cortex/run/<id>.sock` at mode 0600 in the 0700 run folder; on Windows the pipe `cortex-<id>`, whose access list allows the current user's identifier alone (read from `whoami`), refusing remote clients. A connection opens with a Hello; another protocol gets a Mismatch and may still send a Shutdown; then Event (queued for the writer; the moment before a tool runs, and a probe from `cortex doctor`, answered only), Status (version, protocol, process, uptime, what waits, what was lost, each hook's handling time as percentiles of its last 1,000 requests, and the last 10 errors) and Shutdown, which holds even when its acknowledgement cannot be written. A hook's handling time runs from the first byte of its request to its answer being ready; the hook's own process, its connection and the handshake are not in it. At most 64 connections at once; a frame must arrive within 10 s. A connection that breaks (a request that cannot be read, a request before the handshake, a second handshake, an answer that cannot be written) is kept among the recent errors in the daemon's own words, never with the client's bytes, before the client can see the connection end; one closed before a request begins is not an error. An error met again is kept once, with how many times, as the latest |
| (writer) | One inbox feeds the writer: hook events (at most 10,000; when full, a read or search gives way first, and every drop is counted), the watcher's changes, a stop. Events waiting are recorded together in one transaction, all or none; a batch that fails is counted, the store's error is kept among the recent errors, and the daemon goes on. The store opens before the grammars load, and events are recorded every 250 ms while they load and between an index's write transactions, so an event reaches the store within a second whatever the writer is doing. The edits an update of the watcher's changes finds are held 2 s from when the watcher saw them, then recorded after the events waiting (so that a hook's report of the same edit claims it first), for the session most likely to have made them or for none |
| (log) | `~/.cortex/logs/daemon-<id>.log`, started once the daemon holds the repository: one line per event at info level and above (when it started and stopped and why, the first index, each hook request it handled with the request's id, the hook and the time it took, every error it kept for its status, a failed checkpoint of the store, a panic's location, and the error it stopped on), each line the time in UTC milliseconds, the level, the module, the message and its fields. Errors before the log starts (no home folder, the repository held by another daemon) are the caller's to report. A thread of its own writes the lines from a queue of 1,000, so the daemon never waits for the disk; lines that find the queue full are dropped and counted in the log, and a stopping daemon waits at most 2 s for the rest. Rotated at 10 MiB across five files, as the hook's log is; a log that cannot move aside (on Windows, a reader holding it open) keeps taking lines, and the next try comes after another 10 MiB |
| `is_running` | Whether a daemon holds a repository: its lock on `~/.cortex/run/<id>.lock` is taken (tested by taking it for a moment when free) |
| `settings::load` | The configuration in effect for a repository: the user's `~/.cortex/config.toml`, then the repository's `.cortex/config.toml`, each refused over 1 MiB |

## Tests

```sh
cargo test -p cortex-daemon
cargo test -p cortex-cli --test daemon
```

The crate's own tests cover the inbox of changes, the run lock, the configuration files and the
global gitignore lookup; the CLI's daemon and server tests run the real binary on temporary
repositories with an empty home folder, including the endpoint's permissions on each platform.
