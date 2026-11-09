# cortex-coord

Coordination across the sessions of one repository, whichever agents run them. It holds the
rules, and nothing else: where each session is, which sessions an edit concerns, and what each is
told. The daemon asks these rules and gives the text in each agent's own hooks.

- **Positions.** A read holds for 10 minutes; an edit 30 minutes after the session's last edit of
  the node, or 5 minutes after its task ended, whichever comes first; an advisory claim 2 hours. A
  session is live while it had an event in the last 15 minutes. Of a session's two positions on
  one node, the one that holds longer is kept. Every number is a `coordination.*` setting.
- **Concerns.** An edit concerns each other live session holding the edited node, or a node linked
  to it by code (structural weight 0.8 or more) or by history (co-change 0.5 or more), once, by its
  strongest reason. Two sessions editing the same node within 10 minutes are both told first.
- **Notices.** Each side is told in its own words: the editing session at once, the other at its
  next hook that carries text, with the definitions whose signature the edit changed. A notice
  names agents, files, definitions and links, never lines; one whose path is not plain text is not
  given.

## What it must never do

- Lock a file, refuse an edit, or send anything to an agent itself.
- Read the store, the clock or the network: every input is given, so the same inputs give the same
  answer.

## Entry points

- `Position`, `PositionMode`, `Timing` (from `CoordinationConfig`), `is_live`: where a session is
  and until when; `Position::longer_of`, which of two positions is kept.
- `concerned`, `Thresholds` (from `CoordinationConfig`): the sessions an edit concerns, given
  positions, each session's last event and the edited node's links.
- `to_editor`, `to_holder`, `Notice`: what each side is told, with its subject, so that one
  happening told from either side, or in other words, is told to a session once.
- `ago`: how long ago something happened, in whole minutes or hours, as text an agent reads says
  it.

## Tests

```sh
cargo test -p cortex-coord
```

Every rule is tested on hand-built positions and links, the expected answers worked out by hand in
the tests' comments.
