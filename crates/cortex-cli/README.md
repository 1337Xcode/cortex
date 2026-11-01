# cortex-cli

Builds the `cortex` binary: every command a user types and, as the other crates land, the internal
entry points agents and the daemon call.

## What it must never do

- Hold product logic. A command parses its arguments, calls the crate that owns the behaviour and
  prints the result.
- Print command output anywhere but through `src/output.rs`, or panic on a closed pipe. Help,
  `--version` and usage errors are printed by clap.
- Let `cortex hook` print or exit with anything but 0: an agent may block its own tool call on a
  hook's exit code or show its output to the model. `hook` is handed to `cortex-hook` before
  clap sees the command line.

## Entry points

| Command | What it does |
| --- | --- |
| `cortex version`, `cortex --version` | Prints `cortex <version>` |
| `cortex daemon [--root <folder>]` | Keeps the repository's index, the current folder's when no root is given, as its only writer: indexes what changed since the last index, then watches the repository and indexes again after each change. Which files count follows the ignore files, the user's global gitignore among them, which it asks git for (`core.excludesFile` in the user's configuration, else the system's, never the repository's own); when git cannot answer it says so and reads none. In huge mode it parses the files it took inventory of, the changed ones first, `index.crawl_files_per_second` a second on one thread. Writes `cortex: daemon ready: ...` on stderr after the first index and stops after `daemon.idle_shutdown_minutes` without a request or a change once nothing waits to be parsed. A second daemon on the same repository is refused |
| `cortex hook --agent <agent> --event <event>` | What an agent's hook configuration runs. Reads the payload on stdin (at most 1 MiB, ended within 50 ms), turns it into an event, and hands it to the repository's daemon before the event's deadline; when no daemon runs, it starts one in the background, detached, and returns at once. The repository is the nearest folder at or above the payload's working folder that holds `.cortex`, never the home folder or above it; outside one, nothing happens. Whatever goes wrong, it prints nothing and exits 0; failures go to `~/.cortex/logs/hook.log`, at most one line a second, without payload text. It is read before the rest of the command line, so no argument makes it print or fail |
| `cortex status [--root <folder>]` | Asks the repository's daemon for its process, version, uptime and queue (events and changes waiting, events dropped, batches not recorded, checkpoints of the store's log that failed), or says it is not running, not answering or speaks another protocol. A daemon of an older Cortex is first replaced: asked to stop, waited for until it lets go of the repository, and a daemon of this build started in its place (a newer daemon is left alone); then prints whether the first index has finished, the scale mode the last index recorded (and the one the configuration pins, if any), and how many files are stored, parsed and waiting to be parsed, or that the repository has no index. Only reads the store |

Adding a command means one new module in `src/commands/`, one variant of `Command` and one match
arm in `src/commands/mod.rs`.

## Tests

```sh
cargo test -p cortex-cli
```

The tests in `tests/` run the built binary. The daemon tests start it on temporary repositories
with an empty home folder, so the user's own configuration never applies.

Configuration comes from `~/.cortex/config.toml`, then the repository's `.cortex/config.toml`; a
file over 1 MiB is refused, and an unknown key prints a warning.
