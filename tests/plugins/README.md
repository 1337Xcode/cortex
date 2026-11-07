# Plugin recordings

OpenCode, Kilo Code, Amp and OpenClaw run hooks as a plugin's code, not as commands, so Cortex
writes a JavaScript plugin for each that runs `cortex hook`. `record.mjs` runs the golden copy of
each generated plugin (`tests/fixtures/<agent>/plugin-2026-11-06/generated/`) under Node with a
fake host that calls the plugin's hooks as the agent's plugin API describes, and a fake `cortex`.

It checks three things:

- Whatever goes wrong with `cortex` (a run that hangs, fails after answering, answers and hangs,
  answers past 64 KiB, or a program that cannot start), the plugin gives the agent nothing, within
  the event's kill time.
- Every handler, given nothing, empty objects or nulls, returns without throwing.
- Each answer lands where the agent takes text (the system prompt, a tool's output, the user's
  message), and the payloads the plugin sends equal the recorded calls in
  `tests/fixtures/<agent>/plugin-2026-11-06/calls/`, whose working folder is written as `C:\repo`.

```sh
node tests/plugins/record.mjs --check   # what `cargo xtask verify` runs
node tests/plugins/record.mjs           # records the calls again
```

The hook crate's tests check that each generated plugin is its golden copy byte for byte, and
normalize every recorded call. After a change to a plugin under `crates/cortex-hook/plugins/`,
update the golden copies and record the calls again.
