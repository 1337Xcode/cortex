# Notes on this evaluation: the failure map on, and off

Measured on 2026-11-10 with the runner and the `cortex` binary both built at commit `24b19f9`.
Three tasks, 3 runs each, with Claude Code 2.1.318 and `claude-sonnet-5-5`, on the itsdangerous
testbed copied from a folder holding its pinned commit. Every task starts from a regression
committed into the copy (`eval/regressions/`), so the agent's own test run fails. Both arms had
Cortex installed, no task held out, and `mode = "small"` set: itsdangerous has 15 files, which
Cortex treats as a tiny repository, where it gives no text at all (a first pair of runs without the
setting showed exactly that and was not kept). This folder's arm also set
`hints.runtime_grounding = true`; the `2026-11-10-24b19f9-grounding-small-off` arm left it at its
default, off. Nothing else differed.

The question: does naming the repository lines a failed test run went through, beside its output,
shorten the way to a correct fix?

## What this run shows

- The map reached an agent in 1 of the 9 runs with it on (`int-bytes-struct-error`, run 1). In the
  others the agent had read the files its traceback named before it ran the tests, and Python's
  traceback prints each frame's code: the map had nothing to add and stayed silent, as designed.
- Paired by task (`comparison.txt`, this arm against the off arm; its labels "with Cortex" and
  "held out" read here as "map on" and "map off"): every run passed in both arms. Turns of the
  measured session: 0.33 more with the map on (1.00 fewer to 1.33 more); tool calls the same.
  Cumulative input tokens: 13,087 more (551 to 35,190 more), which follows the extra turns more
  than the map's own text: Cortex's texts after tool calls came to 222 tokens over 8 texts with the
  map on and 137 over 6 off.
- These tasks are too short to show an effect: a median of 6 to 7 turns, and the traceback alone
  points at the fault in two of the three.

## What it does not show

Nothing about runners whose frames carry no code (cargo's panics, Go's and Node's frames), where
the map adds a line the output lacks every time, nor about failures that run through files the
agent has not read. `hints.runtime_grounding` stays off by default until an arm with such tasks
shows the effect.
