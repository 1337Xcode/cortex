# Notes on this evaluation

Measured on 2026-11-06 with the runner at commit `e8bee2b` and the `cortex` binary built at commit
`042d163` (no product code changed between them): the 15 tasks of the suite, 2 runs each, with
Claude Code 2.1.318 and `claude-sonnet-5-5`, Cortex installed and no task held out. The other arm,
every task held out, is in `2026-11-06-e8bee2b-held-out`. Each run cloned its testbed at the pinned
commit with its history: `serena` (12 tasks, medium mode) and `itsdangerous` (3 tasks, about 1,700
lines of Python, tiny mode). The runner starts the first session once the index and graph are
ready, which can be before the history is mined: the first sessions may have had no co-change links.

## What the summary shows

- Every run passed. The 15 work orders the measured sessions got were all labelled; 11 were used.
- All 1,003 code lines the sessions were shown were the lines their files held.
- Cortex's texts, read again on every later turn, were at most 1.42% of the sessions' input; on
  the tiny testbed, which gets no work order, at most 0.20% (6 primers and 2 warnings in 6
  sessions).
- Warnings: 75 files named after edits, 6 of them read or edited afterwards (precision 0.08). The
  tests named were never followed up; the call sites named were all in calls the change left
  valid.

## Against the held-out arm: a pilot

Paired by task (the mean of each task's runs in each arm, then the mean difference over the 15
tasks, with a 95% interval from 2,000 resamples of tasks, seed 20261106), treated minus held out:

| Measure, the measured session | Treated | Held out | Difference (95% interval) |
| --- | --- | --- | --- |
| Cumulative input tokens | 212,244 | 184,865 | +27,378 (+428 to +54,230) |
| Turns | 8.8 | 8.7 | +0.17 (-0.73 to +1.10) |
| Tool calls | 7.8 | 7.7 | +0.17 (-0.77 to +1.07) |
| Cost of every session of a run, US dollars | 0.15 | 0.16 | -0.01 (-0.02 to 0.00) |
| Check passed | 1.00 | 0.97 | +0.03 (0.00 to +0.10) |

Fifteen tasks a side is too few to support a claim, and none is made: this run shows no saving.
The sessions given work orders took more input, which the work orders' own tokens, read again on
every turn, do not account for alone; why is the next thing to find out.
