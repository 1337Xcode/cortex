# Notes on this evaluation

Measured commit `c856abc` on 2026-11-05: the 12 tasks of the suite, 4 runs each, with Claude Code
2.1.318 and `claude-sonnet-5-5`, Cortex installed in every run (no task held out). The testbed is
`serena` at its pinned commit, copied from a local folder, so each copy is a git repository of one
commit: Cortex had no history of files changing together to draw on, only the code's structure,
the prompt's words and what the warm-up sessions did.

## The summary was rebuilt

`summary.json` and `summary.txt` here were rebuilt from `runs.json` with `cortex-eval report` at
commit `5485b36`, whose intervals resample whole tasks. The summary the runner wrote when the run
ended gave precision "0.91 (95% interval 0.78 to 1.00)", an interval that drew the 32 work orders
one by one. The four runs of a task share its prompt and code, so they are not independent, and that
interval was too narrow; resampling tasks gives 0.70 to 1.00. `runs.json` is as the runner wrote it.

## By task

| Task | Runs | Check passed | Work orders delivered | Used |
| --- | --- | --- | --- | --- |
| `activation-command-cwd` | 4 | 4 | 4 | 1 |
| `answer-chars-default` | 4 | 4 | 4 | 4 |
| `content-around-line-rename` | 4 | 4 | 4 | 4 |
| `hook-deny-interval-default` | 4 | 4 | 0 | 0 |
| `interprompt-list-fallback` | 4 | 4 | 0 | 0 |
| `ls-settings-get-list` | 4 | 4 | 4 | 4 |
| `memory-exists-method` | 4 | 3 | 4 | 4 |
| `mode-show-command` | 4 | 4 | 4 | 4 |
| `shell-timeout-param` | 4 | 4 | 0 | 0 |
| `task-cancel-pending` | 4 | 4 | 4 | 4 |
| `usage-stats-no-create` | 4 | 4 | 0 | 0 |
| `version-missing-components` | 4 | 4 | 4 | 4 |
| Total | 48 | 47 | 32 | 29 |

Work orders reached the measured session in every run of 8 tasks and in no run of the other 4,
where the evidence did not agree and Cortex added nothing. All 3 unused work orders were in one
task, `activation-command-cwd`.

## What this does not measure

Whether Cortex saves tokens, turns or time: that needs the same tasks with Cortex holding them out,
which this run did not do.
