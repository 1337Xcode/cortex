# Codex CLI 0.166.0 rollout

`transcripts/session.jsonl` is the rollout file Codex CLI 0.166.0 wrote for one headless `codex exec`
session (model `gpt-6-luna`), captured on 2026-11-06 on Windows 11 in a small Python repository: an
eight-step prompt that searched, read, ran the tests, patched a file twice, ran a failing command
and started a subagent. Every line is kept, in order.

## Scrubbing

Applied to the parsed lines, then written back one compact JSON object a line:

- Every UUID, also inside `exec-` ids, is mapped in order of first appearance to a fixed UUID of the
  form `00000000-0000-4000-8000-000000000001`; ids such as `resp_...` and `call_...` become
  `resp_fixture0001` and so on, by kind.
- Every other text is replaced by `scrubbed`, except line and item types, the model's name,
  statuses, sources, kinds, tool names and timestamps; numbers are kept, so every count is as
  Codex wrote it.
- Working folders are `C:\repo`, and a file change's path key keeps only its file name under it.

The `token_usage_record` lines hold one response each (eleven); their `usage` totals add up to the
cumulative `total_token_usage` of the last `token_count` line, 208,979 tokens.
