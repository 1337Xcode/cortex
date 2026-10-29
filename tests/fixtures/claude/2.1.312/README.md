# Claude Code 2.1.312 hook payloads

Real payloads Claude Code 2.1.312 sent to command hooks, captured on 2026-10-29 on Windows 11 from
headless `claude -p` sessions (model Sonnet) in a small Python repository. Every hook was registered
in exec form (`command` plus `args`, no shell) and wrote its stdin to a file unchanged.

One file per event, and per tool for tool events:

| File | Event |
| --- | --- |
| `session-start-{startup,resume,compact,clear}.json` | SessionStart with each `source` seen |
| `user-prompt-submit.json` | UserPromptSubmit |
| `pre-tool-use-<tool>.json`, `post-tool-use-<tool>.json` | Glob, Grep, Read, Bash, Edit, Write, Agent |
| `*-grep-subagent.json` | A tool call made inside a subagent: carries `agent_id` and `agent_type` |
| `post-tool-use-failure-bash.json` | A Bash command that exited non-zero (`error` starts with `Exit code 3`) |
| `pre-compact-manual.json`, `post-compact-manual.json` | `/compact` |
| `subagent-start.json`, `subagent-stop.json` | The Explore subagent |
| `subagent-stop-internal.json` | Claude Code's own compaction agent, with an empty `agent_type` |
| `stop.json`, `session-end.json` | Stop, SessionEnd |
| `session-end-clear.json` | SessionEnd with `reason` `clear` |

During `/compact` the events arrived in this order: SessionStart (`resume`), PreCompact,
SubagentStop (internal), SessionStart (`compact`), PostCompact, SessionEnd.

During `/clear`: SessionStart (`resume`), SessionEnd (`clear`) for the old session, SessionStart
(`clear`) with a new `session_id`, SessionEnd (`other`).

## Scrubbing

Applied to the raw bytes, so key order and spacing are as captured:

- The repository folder is `C:\repo`; the home folder is `C:\Users\dev`; the transcript folder name
  that encodes the repository path is `C--repo`.
- Session, prompt and other UUIDs are mapped, in order of first appearance, to fixed UUIDs of the
  form `00000000-0000-4000-8000-000000000001`, keeping their format. Tool-use ids become
  `toolu_fixture0001` and so on; subagent ids become `fixtureagent0001`.
- Prompts, code and command output come from the synthetic capture repository and contain nothing
  else.
