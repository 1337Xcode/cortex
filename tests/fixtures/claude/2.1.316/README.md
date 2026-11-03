# Claude Code 2.1.316 hook payloads

Real payloads Claude Code 2.1.316 sent to command hooks, captured on 2026-11-01 on Windows 11 from
headless `claude -p` sessions (model Sonnet) in the same small Python repository as the 2.1.312
payloads, with only that repository's settings in effect. Every hook was registered in exec form
(`command` plus `args`, no shell) and wrote its stdin to a file unchanged.

The files and the events they stand for are those of the 2.1.312 folder, one for one, and so are
the event orders during `/compact` and `/clear`. Compared key path by key path, 35 of the 37 hold
the same keys with values of the same types as their 2.1.312 namesakes. The other two,
`pre-tool-use-grep-subagent.json` and `post-tool-use-grep-subagent.json`, differ only in the
arguments the subagent chose for its Grep call (`-n` where the older call gave `glob`), not in
what Claude Code sends.

| File | Event |
| --- | --- |
| `session-start-{startup,resume,compact,clear}.json` | SessionStart with each `source` seen |
| `user-prompt-submit.json` | UserPromptSubmit |
| `pre-tool-use-<tool>.json`, `post-tool-use-<tool>.json` | Glob, Grep, Read, Bash, Edit, Write, Agent |
| `*-grep-subagent.json` | A tool call made inside a subagent: carries `agent_id` and `agent_type` |
| `post-tool-use-grep-folder.json`, `post-tool-use-grep-folder-content.json` | Grep given the folder `calc` as `path`, in files and in content mode: the paths it lists are still relative to the working folder |
| `post-tool-use-grep-one-file.json` | Grep given the one file `calc/ops.py`, in content mode: each line starts with its number and names no file |
| `post-tool-use-grep-one-file-count.json` | Grep given one file in count mode: the line still names the file |
| `post-tool-use-grep-context.json` | Content mode with `-C 1`: context lines `path-N-text`, groups split by `--` |
| `post-tool-use-grep-count.json` | Count mode over the repository: one `path:count` line a file |
| `post-tool-use-bash.json` | A test command piped through `tail` |
| `post-tool-use-failure-bash.json` | A Bash command that exited non-zero (`error` starts with `Exit code 3`) |
| `post-tool-use-bash-test-passing.json` | A test command that passed, not piped |
| `post-tool-use-failure-{bash-test,powershell}.json` | A failing test command through Bash and through PowerShell |
| `*-read-range.json` | Read with `offset` and `limit` |
| `*-powershell.json` | The PowerShell tool, present on Windows beside Bash |
| `pre-compact-manual.json`, `post-compact-manual.json` | `/compact` |
| `subagent-start.json`, `subagent-stop.json` | The Explore subagent |
| `subagent-stop-internal.json` | Claude Code's own compaction agent, with an empty `agent_type` |
| `stop.json`, `session-end.json` | Stop, SessionEnd |
| `session-end-clear.json` | SessionEnd with `reason` `clear` |

During `/compact` the events arrived in this order: SessionStart (`resume`), PreCompact,
SubagentStop (internal), SessionStart (`compact`), PostCompact, SessionEnd. During `/clear`:
SessionStart (`resume`), SessionEnd (`clear`) for the old session, SessionStart (`clear`) with a
new `session_id`, SessionEnd (`other`).

The six Grep files above that hold a folder, one file, context or counts come from a second
session on 2026-11-02, in the same repository and version, allowed the Grep tool alone
(`--allowedTools Grep` instead of skipping permissions), which is why their `permission_mode` is
`default`. Their identifiers are mapped from the first again, so their session id repeats the
other files'.

## Scrubbing

As for the 2.1.312 payloads, applied to the raw bytes, so key order and spacing are as captured:

- The repository folder is `C:\repo`; the home folder is `C:\Users\dev`; the transcript folder name
  that encodes the repository path is `C--repo`.
- Session, prompt and other UUIDs are mapped, in order of first appearance, to fixed UUIDs of the
  form `00000000-0000-4000-8000-000000000001`, keeping their format. Tool-use ids become
  `toolu_fixture0001` and so on; subagent ids become `fixtureagent0001`.
- `harnessSectionHash`, an opaque hash Claude Code reports for its own prompt sections, is replaced
  by zeros of the same length, since what it hashes is not documented.
- Prompts, code and command output come from the synthetic capture repository and contain nothing
  else.

## Transcripts

`transcripts/` holds three of Claude Code 2.1.316's own transcript files from the same repository,
for reading token usage:

| File | What it is |
| --- | --- |
| `grep-session.jsonl` | The second session above: 47 lines, four responses written as seven lines (one per content block), and Claude Code's running totals in a `cost-state` line |
| `hooks-session.jsonl` | The first session, across its several prompts, `/compact` and `/clear`: 168 lines, 11 responses written as 23 lines; the Explore agent's result carries that agent's last usage inside a user line |
| `hooks-session/subagents/agent-fixtureagent0001.jsonl` | The Explore agent's own transcript, every line marked as a sidechain: 19 lines, two responses |

They are reduced as well as scrubbed. Every line is kept, in order, with its `type`. An assistant
line keeps its session, sidechain flag, agent, request id, time, version and ids, and its
message's id, type, role, model, the kinds of its content blocks and its `usage`, as written. A
`cost-state` line keeps `totalCostUSD` and `modelUsage`; a user line keeps the kinds of its
content blocks and a tool result's `usage`; every other line keeps its type, session, ids and
time. Everything else (prompts, tool input and output, the system prompt, request bodies, paths)
is dropped. Ids are mapped in order of first appearance: session and line UUIDs to
`00000000-0000-4000-8000-000000000101` on for the first file and `...000201` on for the other two,
message ids to `msg_fixture0001`, request ids to `req_fixture0001`, the agent to `fixtureagent0001`.
