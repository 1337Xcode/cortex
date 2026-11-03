# cortex-daemon

The process that keeps one repository's index: the store's only writer, the file watcher, in huge
mode the crawler that parses files in the background, and the miner that reads the git history for
the files that change together. `cortex daemon` runs it.

## What it must never do

- Let anything but its writer write the store, or run twice for one repository: the run folder's
  lock refuses a second daemon, and the store's writer lock refuses a second writer.
- Use the network. It asks git for the user's global gitignore with `git config` alone, which reads
  configuration files and nothing else, and reads the history in-process with `gix`, or with the
  `git` program told never to fetch what a partial clone left out.
- Read the repository's own git configuration for that: a repository could then name the file the
  daemon opens. Nor run a `git` the repository holds: the git it asks is the one the `PATH` names
  outside the repository, in an absolute folder.
- Write code, prompt text or tool output to its log. A line holds the daemon's own words, ids,
  counts, times and paths, at most 1,000 characters on one line.

## Entry points

| Item | What it is |
| --- | --- |
| `run` | Takes the repository's hold first (a lock on `~/.cortex/run/<id>.lock`, released by the system when the process ends, and `<id>.pid` naming the process; a second daemon is refused, naming the first's process), then runs the daemon for a repository's canonical root until it has been idle for `daemon.idle_shutdown_minutes` (no request of any kind, no file change) with nothing waiting to be parsed or recorded; configuration warnings and the ready line go to a callback |
| (miner) | A thread of its own, started once the first index is done, that reads HEAD every 2 s. When a mining is due (nothing mined yet, HEAD moved, or the last full mining a week old), it waits one look, so that the watcher reports the files a checkout wrote, and then until the writer has indexed every change that had come by then; it reads the history for the files the store counts and hands what it found to the writer, which stores it in turns with the hook events recorded between them, and waits for that before it looks again. `gix` reads the history, opened for each read and closed after it, so that no file of the repository stays open between looks (on Windows, git cannot remove a file another process holds open); the `git` program the `PATH` names outside the repository reads it once `gix` cannot open or read the repository, and the status says so with `gix`'s reason. Each look then runs git, so the miner looks every 30 s instead while nothing waits for the index. A failure, a panic in a look among them, is reported, logged and kept among the recent errors; the next try waits 2 s, twice as long after each further failure in a row, at most an hour, or only until HEAD moves when HEAD could be read. A miner that cannot start leaves the daemon running without co-change, its status saying so. Told to stop, it ends at its next look; a read under way ends with the process, and a `git` it was running at its next write to the closed pipe or at its time limit |
| (endpoint) | Bound right after the run lock, before the store opens: on Unix `~/.cortex/run/<id>.sock` at mode 0600 in the 0700 run folder; on Windows the pipe `cortex-<id>`, whose access list allows the current user's identifier alone (read from `whoami`), refusing remote clients. A connection opens with a Hello; another protocol gets a Mismatch and may still send a Shutdown; then Event (queued for the writer; the moment before a tool runs answered only, and a probe from `cortex doctor` answered and counted only), Status (version, protocol, process, uptime, what waits, what was lost, the probes answered, each hook's handling time as percentiles of its last 1,000 requests, the last 10 errors, and what the co-change miner is doing) and Shutdown, which holds even when its acknowledgement cannot be written. A hook's handling time runs from the first byte of its request to its answer being ready; the hook's own process, its connection and the handshake are not in it. At most 64 connections at once; a frame must arrive within 10 s. A connection that breaks (a request that cannot be read, a request before the handshake, a second handshake, an answer that cannot be written) is kept among the recent errors in the daemon's own words, never with the client's bytes, before the client can see the connection end; one closed before a request begins is not an error. An error met again is kept once, with how many times, as the latest |
| (writer) | One inbox feeds the writer: hook events (at most 10,000; when full, a read or search gives way first, and every drop is counted), the watcher's changes, a stop. Events waiting are recorded together in one transaction, all or none; a batch that fails is counted, the store's error is kept among the recent errors, and the daemon goes on. The store opens before the grammars load, and events are recorded every 250 ms while they load and between an index's write transactions, so an event reaches the store within a second whatever the writer is doing. The edits an update of the watcher's changes finds are held 2 s from when the watcher saw them, then recorded after the events waiting (so that a hook's report of the same edit claims it first), for the session most likely to have made them or for none; while an index runs, the recorder knows it, so that a hook's edit recorded in one of its pauses takes nothing from a row it wrote as what the file held before. A task that ends (at the next prompt, or the end of its answer or of its session) is learned from once its batch is stored, each task in a transaction of its own with the events waiting recorded after it: what became of it and why, the interest its work earned, the links between what it used, its record, and, now that it has ended, whether the session's task before it was put back; then the newest task records are clustered again. Tasks that end while an index or a mining holds the writer wait for it, at most 256, the oldest let go first. The end of an answer, a subagent's stop and a session's end name a transcript, read once their batch is stored, after the tasks waiting are learned from, from where its last read stopped, in passes of the whole lines within 1 MiB (a longer line read whole, as far as 64 MiB), each pass in a transaction of its own with the events waiting recorded after it, and a transcript with more to read waiting again for the next pass: the tokens each response took become usage rows of the event's session (a subagent's own for its stop), numbered by the response's place in the transcript, so that a transcript read again from its start writes the same rows. A model's name that is not a name (too long, or holding control characters) is stored as no model. A transcript is read only when its path is absolute, its name ends in `.jsonl`, it lies inside a folder its agent keeps transcripts in once every link on the way is followed (for Claude Code `~/.claude/projects`, and the `projects` folder of `CLAUDE_CONFIG_DIR` when the environment the daemon started in sets it to an absolute path), and it is a regular file, opened without following a link; one that is not is logged and kept among the recent errors, in words that name no path. At most 256 transcripts wait, one a path, and 256 have their place kept, the one read longest ago forgotten first; usage the store refuses is counted as a failed batch and its transcript read from its start when an event names it again. However the writer stops (asked to, idle, or on an error once the store is open), it closes the inbox to events, records the events it took and the edits it holds, learns from the tasks waiting and reads the transcripts waiting; an event that arrives after is answered with an error, not acknowledged and lost |
| (log) | `~/.cortex/logs/daemon-<id>.log`, started once the daemon holds the repository: one line per event at info level and above (when it started and stopped and why, the first index, each hook request it handled with the request's id, the hook and the time it took, every error it kept for its status, a failed checkpoint of the store, a panic's location, and the error it stopped on), each line the time in UTC milliseconds, the level, the module, the message and its fields. Errors before the log starts (no home folder, the repository held by another daemon) are the caller's to report. A thread of its own writes the lines from a queue of 1,000, so the daemon never waits for the disk; lines that find the queue full are dropped and counted in the log, and a stopping daemon waits at most 2 s for the rest. Rotated at 10 MiB across five files, as the hook's log is; a log that cannot move aside (on Windows, a reader holding it open) keeps taking lines, and the next try comes after another 10 MiB |
| `is_running` | Whether a daemon holds a repository: its lock on `~/.cortex/run/<id>.lock` is taken (tested by taking it for a moment when free) |
| `settings::load` | The configuration in effect for a repository: the user's `~/.cortex/config.toml`, then the repository's `.cortex/config.toml`, each refused over 1 MiB; only the user's may turn on LLM upkeep |

## Tests

```sh
cargo test -p cortex-daemon
cargo test -p cortex-cli --test daemon
cargo test -p cortex-cli --test learning
cargo test -p cortex-cli --test meter
```

The crate's own tests cover the inbox of changes, the run lock, the configuration files, the
global gitignore lookup, and the tasks learned from as they end: before a stop, after an index's
pause and after the edits that fall due, with the events that wait recorded meanwhile, the work
loop woken while tasks wait, and the bound on how many wait; and the transcripts the stops name:
which events name one, the reading rules (links only on Unix, where they need no privileges), a
transcript named twice read once, a transcript longer than a pass read on in the next, usage the
store refuses read again from the start, refusals kept as one error, the bounds on the transcripts
waiting and kept, and a stop's transcript read before the daemon stops. The CLI's daemon and server tests
run the real binary on temporary repositories with an empty home folder, including the endpoint's
permissions on each platform; its learning test sends captured Claude Code payloads through
`cortex hook` and reads what two tasks of a session left in the store, its meter test sends a
session's stops with captured transcripts in place and reads the usage rows written, and its
durability test checks that events reach the store within a second while the daemon reads a 24 MiB
transcript of short responses.
