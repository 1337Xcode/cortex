# cortex-eval

The evaluation suite: tasks a real coding agent works through in fresh copies of real
repositories, with Cortex installed, and a runner that writes down what the runs took and how
precise Cortex's work orders were. It never writes what a session said: the results hold counts,
the agent's version and model, and where its credentials came from.

## Layout

| Path | Holds |
| --- | --- |
| `testbeds.toml` | Each testbed: a git repository at a pinned commit |
| `tasks/<id>.toml` | One task: its testbed, its kind, the prompts of its warm-up sessions, the prompt of its measured session, the success check, the files a correct solution touches, the tools the agent may use, and any project rules (`[[notes]]`: a rule's text, what it is about as `cortex note --at` takes it, and its kind) |
| `checks/<id>.py` | Each task's success check, run from the repository's root after the sessions; exit code 0 is success |
| `results/<date>-<commit>/` | An evaluation's `runs.json` (each run's counts), `summary.json` and `summary.txt`, named for the day it ran and the commit it measured |

A task's warm-up sessions work in the same area first, as earlier days of work would. The last
session is a new one in that area, and it is the one measured: only the work orders it gets count
for precision.

## Running

```sh
cargo run -p cortex-eval -- check
cargo run -p cortex-eval -- run --arm treated --cortex target/release/cortex \
  --base <a folder outside any repository> --out eval/results/<date>-<commit>
cargo run -p cortex-eval -- report eval/results/<date>-<commit>
cargo run -p cortex-eval -- compare <first arm's folder> <second arm's folder>
```

`compare` pairs two arms' runs by task and writes, to the first arm's folder, each measure's mean
in each arm and the mean per-task difference with its interval: a treated arm against a held-out
one, or an arm that noted the tasks' project rules against one run with `--without-facts`, which
leaves them out and changes nothing else.

`report` rebuilds a results folder's `summary.json` and `summary.txt` from its `runs.json`, so a
change to how runs add up reaches results already made without running their sessions again. It
keeps the runs that could not complete from the summary written when the evaluation ran, and
refuses when that summary is missing.

Each run clones its testbed at the pinned commit, or copies a folder holding it
(`--from serena=<folder>`, leaving `.git/` and `.cortex/` behind), writes the arm's holdout share (0 for `treated`, 1 for `held-out`),
runs `cortex init --yes`, and waits until the index and graph are ready. Unless the arm runs with
`--without-facts`, it then notes the task's project rules with `cortex note`, as a user would, so
they are confirmed before the first session; the summary counts the rules noted and the runs that
left them out. Each session is
`claude -p` with the repository's own settings only (`--setting-sources project,local`), no MCP
server but the repository's, edits accepted and the task's tools allowed, and `ANTHROPIC_API_KEY`
left out of its environment so that Claude Code signs in with its own login. After each session
the runner waits until Cortex has ended its tasks and labelled what it gave; after the last it runs
the check, reads the measured session's injections from the repository's store, and runs
`cortex uninstall`.

Every command the runner waits for writes its output to a file in the run's folder (`init.log`,
`session-<n>.jsonl`, `check.log` and the others), never to a pipe, and is stopped if it runs past
its time. The logs stay with the run, and a file, unlike a pipe, does not depend on what the
processes a command leaves running have inherited.

Each run works in its own folder under `--base`, and the runner refuses to start when any of them
exists already: an evaluation needs a fresh base. A run that cannot complete (a copy, the install,
a session or the store failing) does not stop the others: the runner prints why, and the summary lists it by task and run number, with the completed
runs counted apart.

Before each session the runner keeps the copy's files as they are, through git and an index of
its own, so that the copy's own index and history stay as the agent left them. After the session
it reads the work orders back from the session's transcript, which the store names, and checks
every numbered code line they showed against its file as it was kept. The summary counts the code
lines shown, those that were not their file's line, and the sessions whose transcript could not be
read; `shown.log` in the run's folder names each line that differed by its path and number.

The runner also reads from the store every text Cortex gave each session (the primer, work orders,
warnings, read hints, the recap and strict mode's reasons), by the hook it answered, with its tokens
as Cortex estimates them. A text stays in the agent's context from the turn it arrives, so the
summary weighs each session's text tokens by its turns against the input the sessions report: the
most Cortex can have added to them, since a text that arrives later in a session is read fewer
times.

The warnings every session's edits got are counted by kind, with how many their tasks' ends
labelled and how many were resolved: the file a warning named was read or edited after it. The
warnings' precision is the share of the labelled ones that were resolved, with its interval
resampling whole tasks.

Precision is the share of the measured sessions' work orders that reached the agent, were labelled
when their task ended, and were labelled used (the task edited one of the work order's first three
files, or read one and edited a file that names it). Every share comes with a 95% percentile
bootstrap interval that resamples whole tasks: the runs of one task share its prompt and its code,
so they are not independent, and an interval that drew runs or work orders one by one would be
too narrow.

## Testing

```sh
cargo test -p cortex-eval
```

The tests read tasks and testbeds written in the test, a stream cut to the lines the runner reads,
copy a folder into a repository of one commit, and add up runs worked out by hand.
