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
tasks, with a 95% interval from 2,000 resamples of tasks), treated minus held out, as
`cortex-eval compare` writes it to `comparison.txt` beside this file, which also splits the tasks
by whether their treated runs got a work order:

| Measure, the measured session | Treated | Held out | Difference (95% interval) |
| --- | --- | --- | --- |
| Cumulative input tokens | 212,244 | 184,865 | +27,378 (+2,285 to +56,418) |
| Turns | 8.8 | 8.7 | +0.17 (-0.77 to +1.13) |
| Tool calls | 7.8 | 7.7 | +0.17 (-0.77 to +1.13) |
| Cost of every session of a run, US dollars | 0.151 | 0.158 | -0.008 (-0.021 to +0.005) |
| Check passed | 1.00 | 0.97 | +0.03 (0.00 to +0.10) |

Fifteen tasks a side is too few to support a claim overall, and none is made. Split by work order,
the 8 tasks given one took 1.1 fewer turns (0.44 to 1.88 fewer) and $0.024 less a run (0.009 to
0.041 less); the 7 given none took 1.6 more turns and 70,977 more input tokens (36,986 to 104,311
more), which Cortex's own texts there do not account for: that is the next thing to find out.
