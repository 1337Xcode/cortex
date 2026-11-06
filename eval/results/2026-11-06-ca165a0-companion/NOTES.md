# Notes on this evaluation

Measured on 2026-11-06 with the runner and the `cortex` binary built at commit `ca165a0`: the five
tasks of the suite whose solutions need a change beside the one the prompt names, 2 runs each, with
Claude Code 2.1.318 and `claude-sonnet-5-5`, Cortex installed and no task held out, on `serena`
cloned at its pinned commit with its history. Four tasks rename a parameter that callers in other
files pass by name; one changes a behaviour its module's own test asserts.

## What the summary shows

- Every run passed, and all 10 work orders the measured sessions got were used.
- All 884 code lines the sessions were shown were the lines their files held.
- Warnings after edits: 33 files named, 24 of them read or edited afterwards, a precision of 0.73
  (95% interval 0.45 to 0.96, resampling tasks): 20 of the 22 call sites, 4 of the 9 tests, none
  of the 2 history partners.
- Cortex's texts took at most 5.18% of the sessions' input: these sessions run longer, and their
  edits are warned more.

Five tasks is a small sample; the interval says how far the share could move.
