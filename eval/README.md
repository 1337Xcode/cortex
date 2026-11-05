# cortex-eval

The evaluation suite: tasks a real coding agent works through in fresh copies of real
repositories, with Cortex installed, and a runner that writes down what the runs took and how
precise Cortex's work orders were. It never writes what a session said: the results hold counts,
the agent's version and model, and where its credentials came from.

## Layout

| Path | Holds |
| --- | --- |
| `testbeds.toml` | Each testbed: a git repository at a pinned commit |
| `tasks/<id>.toml` | One task: its testbed, its kind, the prompts of its warm-up sessions, the prompt of its measured session, the success check, the files a correct solution touches, and the tools the agent may use |
| `checks/<id>.py` | Each task's success check, run from the repository's root after the sessions; exit code 0 is success |
| `results/<date>/` | A run's `runs.json` (each run's counts), `summary.json` and `summary.txt` |

A task's warm-up sessions work in the same area first, as earlier days of work would. The last
session is a new one in that area, and it is the one measured: only the work orders it gets count
for precision.

## Running

```sh
cargo run -p cortex-eval -- check
cargo run -p cortex-eval -- run --arm treated --cortex target/release/cortex \
  --base <a folder outside any repository> --out eval/results/<date>
```

Each run clones its testbed at the pinned commit, or copies a folder holding it
(`--from serena=<folder>`), writes the arm's holdout share (0 for `treated`, 1 for `held-out`),
runs `cortex init --yes`, and waits until the index and graph are ready. Each session is
`claude -p` with the repository's own settings only (`--setting-sources project,local`), no MCP
server but the repository's, edits accepted and the task's tools allowed, and `ANTHROPIC_API_KEY`
left out of its environment so that Claude Code signs in with its own login. After each session
the runner waits until Cortex has ended its tasks and labelled what it gave; after the last it runs
the check, reads the measured session's injections from the repository's store, and runs
`cortex uninstall`.

Every command the runner waits for writes its output to a file in the run's folder (`init.log`,
`session-<n>.jsonl`, `check.log` and the others), never to a pipe, and is stopped if it runs past
its time. The daemon that `cortex init` or a hook starts outlives the command, and on Windows it
would keep a pipe it inherited open, so a reader waiting for the pipe to end would wait as long as
the daemon runs.

Precision is the share of the measured sessions' work orders that reached the agent, were labelled
when their task ended, and were labelled used (the task edited one of the work order's first three
files, or read one and edited a file that refers to it). Every share comes with a 95% percentile
bootstrap interval.

## Testing

```sh
cargo test -p cortex-eval
```

The tests read tasks and testbeds written in the test, a stream cut to the lines the runner reads,
copy a folder into a repository of one commit, and add up runs worked out by hand.
