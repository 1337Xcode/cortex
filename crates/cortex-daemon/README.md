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

## Entry points

| Item | What it is |
| --- | --- |
| `run` | Takes the repository's hold first (a lock on `~/.cortex/run/<id>.lock`, released by the system when the process ends, and `<id>.pid` naming the process; a second daemon is refused, naming the first's process), then runs the daemon for a repository's canonical root until it has been idle for `daemon.idle_shutdown_minutes` with nothing waiting to be parsed; configuration warnings and the ready line go to a callback |
| (endpoint) | Bound right after the run lock, before the store opens: on Unix `~/.cortex/run/<id>.sock` at mode 0600 in the 0700 run folder; on Windows the pipe `cortex-<id>`, whose access list allows the current user's identifier alone (read from `whoami`), refusing remote clients. A connection opens with a Hello; another protocol gets a Mismatch and may still send a Shutdown; then Event (queued for the writer, the moment before a tool runs answered only), Status (version, protocol, process, uptime, what waits, what was lost) and Shutdown. At most 64 connections at once; a frame must arrive within 10 s |
| (writer) | One inbox feeds the writer: hook events (at most 10,000; when full, a read or search gives way first, and every drop is counted), the watcher's changes, a stop. Events waiting are recorded together in one transaction, all or none; a batch that fails is counted and the daemon goes on |
| `settings::load` | The configuration in effect for a repository: the user's `~/.cortex/config.toml`, then the repository's `.cortex/config.toml`, each refused over 1 MiB |

## Tests

```sh
cargo test -p cortex-daemon
cargo test -p cortex-cli --test daemon
```

The crate's own tests cover the inbox of changes, the run lock, the configuration files and the
global gitignore lookup; the CLI's daemon and server tests run the real binary on temporary
repositories with an empty home folder, including the endpoint's permissions on each platform.
