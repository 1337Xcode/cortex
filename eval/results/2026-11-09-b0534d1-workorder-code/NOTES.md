# Notes on this evaluation: how much a work order should show

Measured on 2026-11-09 with the runner and the `cortex` binary both built at commit `b0534d1`.
Every task of the suite, 2 runs each, with Claude Code 2.1.318 and `claude-sonnet-5-5`, each
testbed cloned at its pinned commit with its history. All three arms had Cortex installed and no
task held out; only the work order's shape changed.

The question: does showing code lines in a work order save the agent reads and turns, or does the
agent read the files anyway, so that the code lines only add tokens?

| Folder | Work order | Runs passed | Cost of every session |
| --- | --- | --- | --- |
| `2026-11-09-b0534d1-workorder-code` | As shipped: up to 5 files, up to 2 spans shown as code | 39 of 39 completed (1 of 40 could not complete) | $6.16 |
| `2026-11-09-b0534d1-workorder-pointers` | Files, line ranges and reasons only, no code (`retrieval.max_code_spans = 0`) | 40 of 40 | $6.37 |
| `2026-11-09-b0534d1-workorder-files3` | At most 3 files (`retrieval.max_files = 3`) | 40 of 40 | $6.23 |

## What this run shows

- In each arm the measured sessions received 33 or 34 work orders and the agent used 29 or 30
  (precision 0.88). Every code line shown (2,147 and 2,428 in the two arms that show code) was its
  file's line when shown.
- The text Cortex added at prompts came to 40,990 tokens across the code arm's sessions, 8,645
  across the pointers arm's and 42,248 across the files3 arm's, at most 2.95%, 0.97% and 2.66% of
  the sessions' input.
- Pointers against code, paired by task (`comparison.txt` in the pointers folder; 20 tasks, means
  over tasks, 95% intervals from resampling tasks): 603 fewer uncached input tokens a session (444
  to 759 fewer); 10,149 fewer cumulative input tokens (23,526 fewer to 3,648 more); turns 0.07
  higher (0.88 fewer to 0.93 more); cost $0.001 less a run ($0.011 less to $0.008 more).
- At most 3 files against code (`comparison.txt` in the files3 folder): 57 more cumulative input
  tokens (20,322 fewer to 28,889 more); 0.50 fewer turns (1.45 fewer to 0.40 more). On the 17
  tasks that got a work order, 0.91 fewer turns (1.97 fewer to 0.09 fewer) and 13,451 fewer
  cumulative input tokens (28,521 fewer to 801 more).
- In `comparison.txt`, "with Cortex" names the folder's own arm and "held out" names the code arm:
  both arms had Cortex, and the labels are the comparison tool's.

## What was decided

The work order keeps its shipped shape. The rule set before the run was to change the defaults only
if an arm cut the measured session's cumulative input tokens with no run lost, and neither interval
excludes no change. Removing code lines removed their own tokens and changed no measured turn or
total; showing code did not measurably save turns. A larger run is needed to settle it either way.

## How it was run

`cortex-eval run --arm treated --runs 2`, once per arm, the pointers and files3 arms with `--set`
for their key; then `cortex-eval compare` of each against the code arm, which wrote
`comparison.json` and `comparison.txt` in that arm's folder. `runs.json` holds every run's
measurements and `summary.txt` the arm's totals. One run of the code arm did not settle within the
runner's wait and is counted as not completed.
