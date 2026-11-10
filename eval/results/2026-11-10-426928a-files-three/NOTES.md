# Notes on this evaluation: work orders of three files against five

Measured on 2026-11-10 with the runner and the `cortex` binary both built at commit `426928a`. The
serena testbed's 20 tasks, 3 runs each, with Claude Code 2.1.318 and `claude-sonnet-5-5`, Cortex
installed in both arms and no task held out. This folder's arm set `retrieval.max_files = 3`; the
`2026-11-10-426928a-files-five` arm left it at its default, 5. Nothing else differed. The two arms
ran one after the other on the same machine.

The question: does a work order listing at most 3 files, its freed budget going to code, save
turns or input against one listing 5? An earlier comparison across two builds suggested it might.

## What this run shows

- Paired by task (`comparison.txt`; its labels "with Cortex" and "held out" read here as "three
  files" and "five files"), on the 15 tasks both arms completed: every run passed in both arms.
  With three files the measured session took 0.89 more turns (0.29 to 1.67 more), 0.89 more tool
  calls (0.22 to 1.64 more) and 18,082 more cumulative input tokens (2,239 to 34,850 more);
  uncached input was the same (21 fewer, 69 fewer to 54 more). Each interval is 95%, resampling
  tasks.
- On the 12 paired tasks whose runs got a work order, the same direction: 1.06 more turns (0.31
  to 1.89 more) and 18,388 more cumulative input tokens (551 to 38,665 more).
- The work orders' files earn their place: with fewer of them listed, the agent found the rest
  itself, a turn and a tool call more a task. `retrieval.max_files` stays at 5.

## What it does not show

- Five of the 20 tasks are not in the comparison. `ls-settings-get-list` and
  `version-missing-components` did not settle in either arm (their sessions ended, but the runner
  built here waited for texts a task's end never labels; fixed in the runner since). In the
  five-file arm, the runs of `task-cancel-pending`, `text-coordinates-unique-only`,
  `usage-stats-no-create` and `version-missing-components` could not start: `git init` exited
  0xc0000142, a process Windows could not start while other builds ran beside the arm.
- Nothing about other repositories, other agents or other models.
