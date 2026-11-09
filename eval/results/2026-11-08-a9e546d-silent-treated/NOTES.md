# Notes on this evaluation

Measured on 2026-11-08 with the runner and the `cortex` binary both built at commit `a9e546d`: the
7 tasks of the suite that got no work order in the 2026-11-06 pilot, 2 runs each, with Claude Code
2.1.318 and `claude-sonnet-5-5`, each testbed cloned at its pinned commit with its history. Three
arms, each in a folder of its own:

| Folder | Arm | Runs passed | Cost of every session |
| --- | --- | --- | --- |
| `2026-11-08-a9e546d-silent-treated` | Cortex installed, no task held out | 14 of 14 | $1.92 |
| `2026-11-08-a9e546d-silent-nowarn` | The same with `warnings.max_per_edit = 0` (no warnings after edits) | 14 of 14 | $1.95 |
| `2026-11-08-a9e546d-silent-held-out` | Cortex installed, every task held out | 14 of 14 | $2.07 |

The question was why, in the pilot, these 7 tasks took 70,977 more input tokens and 1.6 more turns
with Cortex than held out, though Cortex gave them no work order. In the pilot, the runs that got
warnings after edits took the most extra turns, so the warnings were the first suspect.

## What this run shows

Paired by task, as `cortex-eval compare` writes it to each treated folder's `comparison.txt`
(means over tasks, 95% intervals from resampling tasks), against the held-out arm:

| Measure, the measured session | With Cortex | Warnings off | Held out |
| --- | --- | --- | --- |
| Cumulative input tokens | 218,940 | 222,028 | 238,647 |
| Turns | 9.21 | 9.50 | 9.86 |
| Cost of every session of a run, US dollars | 0.137 | 0.140 | 0.148 |

- With Cortex against held out: 19,707 fewer input tokens (95,424 fewer to 34,881 more), 0.64
  fewer turns (2.43 fewer to 0.86 more), $0.011 less a run (0.026 less to 0.001 more).
- Warnings off against held out: 16,619 fewer input tokens (70,499 fewer to 24,074 more), 0.36
  fewer turns (1.64 fewer to 0.64 more), $0.008 less a run (0.018 less to 0.003 more).

The pilot's extra cost on these tasks did not come back on this build: the difference it showed is
not a steady effect of Cortex, and two runs a task leave the agent's own spread in it. Turning the
warnings off changed nothing measurable here either.

The treated arm's edits got 18 warnings (6 call sites, 6 co-changes, 6 tests), none of whose
files was read or edited afterwards (precision 0.00); Cortex's texts were at most 1.01% of the
sessions' input. The warnings arm's runs record `warnings.max_per_edit = 0` in `runs.json`.
