# Claude Code 2.1.318 hook payloads

Real payloads Claude Code 2.1.318 sent to command hooks, captured on 2026-11-08 on Windows 11 from
headless `claude -p` sessions (model Sonnet) in the same small Python repository as the 2.1.312 and
2.1.316 payloads, with only that repository's settings in effect. Every hook was registered in exec
form (`command` plus `args`, no shell) and wrote its stdin to a file unchanged.

The files and the events they stand for are those of the 2.1.316 folder, one for one, and so are
the event orders during `/compact` and `/clear`. Compared key path by key path, 37 of the 43 hold
the same keys with values of the same types as their 2.1.316 namesakes. The other six:

| Files | What differs |
| --- | --- |
| `pre-tool-use-agent.json`, `post-tool-use-agent.json` | The Explore subagent ran in the background: the call has no `run_in_background` key, and its result says `isAsync` with an `outputFile` instead of the agent's text, usage and tool counts |
| `post-tool-use-bash-test-passing.json`, `post-tool-use-failure-bash-test.json` | The Bash call carries a `description`, which the model chose to give |
| `stop.json`, `subagent-stop.json` | A `background_tasks` list naming the background agent: its id, type, agent type, description and status |

Cortex reads none of these keys: the Agent tool is a tool of another kind to it, whose response it
does not read, and a stop's fields it reads are unchanged.

The payloads come from four sessions, all in that repository: the first used each tool the
fixtures cover in turn, then was resumed, compacted (`/compact`) and cleared (`/clear`); the second
was allowed the Grep tool alone and made the six Grep calls whose files name a folder, one file,
context or counts; the third, allowed Bash alone, ran the two `unittest` test commands; the fourth,
allowed Write alone, created `notes.txt`, since the first session's file already existed when it
wrote. Sessions allowed one tool run with `permission_mode` `default`. Identifiers are mapped across
all four in order of first appearance.

| File | Event |
| --- | --- |
| `session-start-{startup,resume,compact,clear}.json` | SessionStart with each `source` seen |
| `user-prompt-submit.json` | UserPromptSubmit |
| `pre-tool-use-<tool>.json`, `post-tool-use-<tool>.json` | Glob, Grep, Read, Bash, Edit, Write, Agent |
| `*-grep-subagent.json` | A tool call made inside a subagent: carries `agent_id` and `agent_type` |
| `post-tool-use-grep-folder.json`, `post-tool-use-grep-folder-content.json` | Grep given the folder `calc` as `path`, in files and in content mode |
| `post-tool-use-grep-one-file.json` | Grep given the one file `calc/ops.py`, in content mode |
| `post-tool-use-grep-one-file-count.json` | Grep given one file in count mode |
| `post-tool-use-grep-context.json` | Content mode with `-C 1` |
| `post-tool-use-grep-count.json` | Count mode over the repository |
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

## Scrubbing

As for the 2.1.312 and 2.1.316 payloads, applied to the raw bytes, so key order and spacing are as
captured: the repository folder is `C:\repo`, the home folder `C:\Users\dev`, the transcript folder
name `C--repo`; UUIDs, tool-use ids and subagent ids are mapped to fixed ones in order of first
appearance; `harnessSectionHash` is replaced by zeros; the address in a commit trailer that
Claude Code's instructions give, which the compaction summary repeats, is removed; prompts, code and command output come from
the synthetic capture repository and contain nothing else.
