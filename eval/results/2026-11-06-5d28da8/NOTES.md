# Notes on this evaluation

Measured on 2026-11-06 with the runner at commit `e8bee2b` and the `cortex` binary built from
commit `5d28da8`, whose warnings name only a file's own tests and partners that hold from both
sides: the 17 tasks of the suite, 2 runs each, with Claude Code 2.1.318 and `claude-sonnet-5-5`,
Cortex installed and no task held out. The testbeds were cloned at their pinned commits with their
history: `serena` (14 tasks) and `itsdangerous` (3 tasks, tiny mode). Two of the tasks are new and
need a companion change to pass: renaming a parameter its callers pass, and changing a behaviour its
own test asserts.

## What the summary shows

- Every run passed. The measured sessions got 18 work orders, all labelled; 15 were used.
- All 1,300 code lines the sessions were shown were the lines their files held.
- Cortex's texts, read again on every later turn, were at most 1.71% of the sessions' input.
- Warnings: 32 files named after edits (75 in the run before the rules changed, on 15 tasks), 4
  of them read or edited afterwards, a precision of 0.12 (95% interval 0.00 to 0.33). The call
  sites a renamed parameter left were followed up; most tests named were of edits that needed no
  test change, and were left.

Warnings are far less often followed up than work orders are used. On past commits of two Python
projects replayed through the same hooks, a warning named a file the commit went on to change 62%
of the time on one and 13% on the other; in these tasks most edits did not need the file a
warning named.
