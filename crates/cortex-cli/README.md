# cortex-cli

Builds the `cortex` binary: every command a user types and, as the other crates land, the internal
entry points agents and the daemon call.

## What it must never do

- Hold product logic. A command parses its arguments, calls the crate that owns the behaviour and
  prints the result.
- Print command output anywhere but through `src/output.rs`, which escapes control characters so
  that a path or another program's answer cannot drive the terminal, or panic on a closed pipe. Help,
  `--version` and usage errors are printed by clap.
- Let `cortex hook` print or exit with anything but 0: an agent may block its own tool call on a
  hook's exit code or show its output to the model. `hook` is handed to `cortex-hook` before
  clap sees the command line.

## Entry points

Without `--root`, `status` and `doctor` work on the repository the current folder is in, found as
a hook finds it (the nearest folder at or above it that holds `.cortex`, never the home folder),
and otherwise on the current folder; `init` starts from the current folder.

| Command | What it does |
| --- | --- |
| `cortex version`, `cortex --version` | Prints `cortex <version>` |
| `cortex daemon [--root <folder>]` | Keeps the repository's index, the current folder's when no root is given, as its only writer: indexes what changed since the last index, then watches the repository and indexes again after each change. Which files count follows the ignore files, the user's global gitignore among them, which it asks git for (`core.excludesFile` in the user's configuration, else the system's, never the repository's own); when git cannot answer it says so and reads none. In huge mode it parses the files it took inventory of, the changed ones first, `index.crawl_files_per_second` a second on one thread. Writes `cortex: daemon ready: ...` on stderr after the first index and stops after `daemon.idle_shutdown_minutes` without a request or a change once nothing waits to be parsed. A second daemon on the same repository is refused |
| `cortex hook --agent <agent> --event <event>` | What an agent's hook configuration runs. Reads the payload on stdin (at most 1 MiB, ended within 50 ms), turns it into an event, and hands it to the repository's daemon before the event's deadline; when no daemon runs, it starts one in the background, detached, and returns at once. The repository is the nearest folder at or above the payload's working folder that holds `.cortex`, never the home folder or above it; outside one, nothing happens. Whatever goes wrong, it prints nothing and exits 0; failures go to `~/.cortex/logs/hook.log`, at most one line a second, without payload text. It is read before the rest of the command line, so no argument makes it print or fail |
| `cortex status [--root <folder>]` | Asks the repository's daemon for its process, version, uptime and queue (events and changes waiting, events dropped, batches not recorded, checkpoints of the store's log that failed), how long the daemon itself took to handle each hook (the median, 95th and 99th percentiles of each hook's last 1,000 requests, in milliseconds, from the first byte of a request to its answer being ready: the hook's own process and connection are not counted) and its latest errors (the last 10, each with how long ago it last happened and how many times), or says it is not running, not answering or speaks another protocol. A daemon of an older Cortex is first replaced: asked to stop, waited for until it lets go of the repository, and a daemon of this build started in its place (a newer daemon is left alone); then prints whether the first index has finished, the scale mode the last index recorded (and the one the configuration pins, if any), and how many files are stored, parsed and waiting to be parsed, or that the repository has no index. Only reads the store |
| `cortex init [--root <folder>] [--scope <scope>]` | Sets Cortex up in the top of the git repository that holds the folder it starts from, as `git rev-parse --show-toplevel` says (git found on the `PATH` outside every repository around the folder), and prints which; in that folder alone, with a note saying why, when git is not on the `PATH`, names no repository, names a top that does not hold the folder (a repository's `core.worktree` can name any folder), or names the home folder or a folder holding it. It never sets up the home folder itself or a folder holding it, whose `.cortex` is the user's own. Asks Claude Code its version as `doctor` does and refuses, before writing anything, when it is older than 2.1.295. With `--scope user` it runs no git. Then registers Cortex's hooks with Claude Code: in the repository's `.claude/settings.local.json` (scope `project`, the default), or in `~/.claude/settings.json` (scope `user`). Each event Cortex records gets one exec-form hook: this binary run with `hook --agent claude --event <event>`, the event's tool matcher, its hard timeout in whole seconds rounded up, and `onFailure: "continue"`. Cortex's own hooks, found by a command named `cortex` whose arguments begin `hook --agent claude`, are replaced where the first of them was; the user's hooks and keys stay, in their order, and running it again changes nothing. Before writing, it copies the file as it was read to a backup folder of the run's own, `.cortex/backups/<UTC milliseconds>-<process>/` in the repository (in `~/.cortex/backups/` for the user's file, which makes nothing in the repository), with a manifest naming the file and the BLAKE3 hash of what it wrote; then it writes a new file beside the old one, made for this write and never through a link, and renames it over the old one only if the old one still holds what was read. A file that is not JSON, holds more than 1 MiB, is read-only or is a link to nothing is refused and left as it is, and so is a repository whose `.claude` or `.cortex` folder is a link or junction leading elsewhere |
| `cortex doctor [--root <folder>]` | Checks, one line each, whether Claude Code will run Cortex's hooks for the repository and the daemon will answer them in time: the hooks are in exactly one settings file of the repository's or the user's and are what this build writes; no settings file it can read (the managed ones, the repository's local and shared ones, the user's) turns hooks off, by `disableAllHooks` as the most precedent file setting it says, or by a managed `allowManagedHooksOnly`; the hooks run this binary (it never runs a program a settings file names); Claude Code, asked `claude --version` (the program on the `PATH` as a shell finds it, never one inside the repository, for at most 5 s), is at least 2.1.295, the oldest the hooks work with (one newer than 2.1.312, the newest tested, is a note); the repository's daemon answers and runs this build; and, while it does, 20 probes of each hook with a budget (SessionStart 120 ms, UserPromptSubmit 150 ms, PreToolUse on a read 40 ms, PostToolUse on an edit 80 ms, Stop 60 ms), this binary run as Claude Code runs it on payloads marked as probes, all reach the daemon, which answers and counts them and records none, and each hook's 95th percentile from start to exit is within its budget. Each failure prints its fix, a last line names what it cannot see (`--safe-mode`, `--settings`, settings an organization delivers by server, MDM or registry, an untrusted folder), and the command fails when any check did. Doctor writes nothing and starts or stops no daemon, and neither does a probe |

Adding a command means one new module in `src/commands/`, one variant of `Command` and one match
arm in `src/commands/mod.rs`.

## Tests

```sh
cargo test -p cortex-cli
cargo bench -p cortex-cli --bench hooks
```

The tests in `tests/` run the built binary. The daemon tests start it on temporary repositories
with an empty home folder, so the user's own configuration never applies.

The `hooks` benchmark runs the built `cortex hook` once per sample, 100 samples per event, from
the process's start to its exit: SessionStart, UserPromptSubmit, PreToolUse on a read, PostToolUse
on an edit and Stop, each on a recorded Claude Code payload against a daemon on a small temporary
repository, and a hook given no input, which ends before it asks the daemon anything. It fails
when a hook fails or when the daemon did not handle each event's requests.

Configuration comes from `~/.cortex/config.toml`, then the repository's `.cortex/config.toml`; a
file over 1 MiB is refused, and an unknown key prints a warning.
