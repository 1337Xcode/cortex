# Codex CLI 0.165.0 hook payloads

Real payloads Codex CLI 0.165.0 sent to command hooks, captured on 2026-10-29 on Windows 11 from
two headless `codex exec` sessions (model `gpt-6-luna`) in a small Python repository. User
configuration was ignored and each hook was passed on the command line (`-c hooks.<Event>=[...]`);
every hook wrote its stdin to a file unchanged. The second session set
`model_auto_compact_token_limit` low, so reading a long file triggered an automatic compaction in
the middle of the turn.

One file per event, and per tool or outcome for tool events:

| File | Event |
| --- | --- |
| `session-start-{startup,compact}.json` | SessionStart with each `source` seen |
| `user-prompt-submit.json` | UserPromptSubmit |
| `pre-tool-use-bash-*.json`, `post-tool-use-bash-*.json` | Shell commands: a search (`git grep`), a read (`Get-Content`), the test command failing and then passing, and a command that exited with status 3 |
| `*-apply-patch*.json` | File edits through `apply_patch`: an updated file and an added file |
| `*-spawn-agent.json`, `*-wait-agent.json` | Starting a subagent and waiting for it |
| `*-bash-subagent.json` | A command run inside the subagent: carries `agent_id` and `agent_type` |
| `pre-compact-auto.json`, `post-compact-auto.json` | Automatic compaction |
| `subagent-start.json`, `subagent-stop.json` | The subagent |
| `stop.json`, `session-end.json` | Stop, SessionEnd |

What the payloads show:

- Every shell command arrives as tool `Bash`, also on Windows, where Codex runs it with PowerShell.
  `tool_response` is the command's output as one string and carries no exit status: in
  `post-tool-use-bash-nonzero-exit.json` the command exited with status 3 and the response is
  empty. Command output keeps Windows line endings inside the JSON strings; the files use LF.
- `apply_patch` puts the whole patch in `tool_input.command`. Its hunks (`@@`) carry context lines,
  not line numbers. Its response is text that starts with `Exit code: 0`.
- Subagent tools carry their namespace with no separator: `collaborationspawn_agent`,
  `collaborationwait_agent`.
- A subagent's events keep the parent's `session_id`. The subagent's `agent_id` also names its own
  transcript file, which is the `transcript_path` of its tool events and the
  `agent_transcript_path` of SubagentStop.
- During the automatic compaction the events arrived in this order: PreCompact, PostCompact,
  SessionStart (`compact`), then the next model request. In that session the SessionStart
  (`compact`) hook answered with `additionalContext`, and Codex added the text to the conversation
  before the next request; the same answer from the PostCompact hook was dropped.

## Scrubbing

Applied to the raw bytes, so key order and spacing are as captured:

- The repository folder is `C:\repo`; the home folder is `C:\Users\dev`.
- Session, turn, subagent and command ids are mapped, in order of first appearance, to fixed UUIDs
  of the form `00000000-0000-4000-8000-000000000001`, keeping their format, so command ids keep
  their `exec-` prefix. Function call ids become `call_fixture0001` and so on.
- `spawn_agent` passes an opaque encrypted token in `tool_input.message`; it is replaced by
  `gAAAAA_fixture_encrypted_message`.
- Prompts, code and command output come from the synthetic capture repository and contain nothing
  else.
