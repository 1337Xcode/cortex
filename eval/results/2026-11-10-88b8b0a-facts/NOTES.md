# Notes on this evaluation: project rules noted, and left out

Measured on 2026-11-10 with the runner and the `cortex` binary both built at commit `88b8b0a`.
Three tasks, 3 runs each, with Claude Code 2.1.318 and `claude-sonnet-5-5`, the serena testbed
copied from a folder holding its pinned commit. Both arms had Cortex installed and no task held
out. In this folder's arm each run noted its task's project rule with `cortex note` before the
first session; the `2026-11-10-88b8b0a-nofacts` arm ran with `--without-facts`, which leaves the
rules out and changes nothing else.

The question: does a project rule the code does not show at the edit site, given in the work order
or the session's primer, change whether the task succeeds and what the session takes?

Each task is decided by such a rule:

| Task | The rule | Passed with the rule | Passed without |
| --- | --- | --- | --- |
| `heartbeat-timeout-setting` | A new `SerenaConfig` setting is documented in the global configuration template | 3 of 3 | 2 of 3 |
| `ls-shutdown-timeout-setting` | A new `ProjectConfig` setting is documented in the project template | 3 of 3 | 3 of 3 |
| `health-check-url-changelog` | A user-visible fix gets a changelog entry | 3 of 3 | 3 of 3 |

## What this run shows

- Paired by task (`comparison.txt`; 3 tasks, means over tasks, 95% intervals from resampling
  tasks): the check passed in 1.00 of runs with the rule and 0.89 without, a difference of 0.11
  (0.00 to 0.33). Cumulative input tokens of the measured session: 8,563 more with the rule
  (68,443 fewer to 69,262 more). Uncached input tokens: 87 more (15 to 140 more). Turns and tool
  calls: 1.22 more each (2.00 fewer to 3.00 more). Cost of every session of a run: $0.006 more
  ($0.022 less to $0.026 more).
- The 87 uncached tokens are about the rules' own text: Cortex's texts at prompts came to 9,234
  tokens over 15 texts with the rules and 8,895 without, and at session starts to 1,086 tokens over
  18 texts with the rules and 816 without. The changelog rule is about the whole repository, so it
  goes in the primer: the first session of that task's first run started with 87 tokens from
  Cortex with the rule and 42 without.
- Every code line shown (459 in each arm) was its file's line when shown.
- Two of the three tasks passed in every run of both arms: the agent documented the project
  setting and wrote the changelog entry without being told. Only the heartbeat task separates the
  arms, by one run. Three tasks are too few to say the rules help; they show that, here, they did
  not make a run fail or cost measurably more.

## Not measured here

Whether a rule changes behaviour on tasks where the agent would otherwise miss it often: such
tasks are hard to write against a repository whose own guidance files already state most of its
conventions.
