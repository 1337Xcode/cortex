# Notes on this evaluation

Measured on 2026-11-09 with the runner and the `cortex` binary both built at commit `a4bd16f`, the
build whose gate lets a work order through when three kinds of evidence agree even if the prompt's
words match the code weakly. Every task of the suite, 2 runs each, with Claude Code 2.1.318 and
`claude-sonnet-5-5`, each testbed cloned at its pinned commit with its history.

| Folder | Arm | Runs passed | Cost of every session |
| --- | --- | --- | --- |
| `2026-11-09-a4bd16f-treated` | Cortex installed, no task held out | 40 of 40 | $6.44 |
| `2026-11-09-a4bd16f-held-out` | Cortex installed, every task held out | 39 of 40 | $6.60 |

## What this run shows

- Work orders reached the measured sessions 34 times and the agent used 30 of them (precision
  0.88). 17 of the 20 tasks got one.
- Every one of the 2,228 code lines shown to the agent was its file's line when shown.
- Paired by task (`comparison.txt`, means over tasks, 95% intervals from resampling tasks): 0.55
  fewer turns and tool calls with Cortex (1.32 fewer to 0.15 more), $0.004 less a run ($0.014 less
  to $0.005 more), 5,383 fewer cumulative input tokens (28,112 fewer to 16,318 more), and every
  run passed against 39 of 40 held out.
- What Cortex adds costs 897 more uncached input tokens a session (689 to 1,097 more), its work
  orders at most 3.06% of the sessions' input.

## How it was run

`cortex-eval run --arm treated` and `--arm held-out`, each `--runs 2`, then
`cortex-eval compare` on the two folders, which wrote `comparison.json` and `comparison.txt` here.
`runs.json` holds every run's measurements; `summary.txt` the arm's totals.
