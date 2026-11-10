# cortex-coord

Coordination across the sessions of one repository, whichever agents run them. It holds the
rules, and nothing else: where each session is, which sessions an edit concerns, and what each is
told. The daemon asks these rules and gives the text in each agent's own hooks.

- **Positions.** A read holds for 10 minutes; an edit 30 minutes after the session's last edit of
  the node, or 5 minutes after its task ended, whichever comes first; an advisory claim 2 hours. A
  session is live while it had an event in the last 15 minutes. Of a session's two positions on
  one node, the one that holds longer is kept, except that a claim still holding is kept against
  any read or edit. Every number is a `coordination.*` setting.
- **Concerns.** An edit concerns each other live session holding the edited node, or a node linked
  to it by code (structural weight 0.8 or more) or by history (co-change 0.5 or more), once, by its
  strongest reason. Two sessions editing the same node within 10 minutes are both told first.
- **Notices.** Each side is told in its own words: the editing session at once, the other at its
  next hook that carries text, with the definitions whose signature the edit changed. A notice
  names agents, files, definitions and links, never lines; one whose path is not plain text is not
  given.
- **Contracts.** A notice to a session whose file calls definitions whose signature the edit
  changed can say each signature before and after and the lines that call it, at most three
  definitions and three lines each, the rest counted. A signature is the header after the
  definition's name, whitespace collapsed, at most 120 characters (a longer one is cut after a
  parameter with ` ...`), never holding a backquote, a control or hidden character or a
  reminder tag, and it is shown as code, in backquotes. The caller gives only signatures and
  lines it checked against the files on disk.

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
- `Signature`, `MAX_SIGNATURE_CHARS`: a definition's signature as a notice may say it, read from
  its declaration header, or none.
- `contract_to_holder`, `Called`, `Holding`: the notice a session holding a file is told when
  another changed the signatures of definitions the file calls, with both signatures and the
  calling lines.
- `ago`: how long ago something happened, in whole minutes or hours, as text an agent reads says
  it.
- `handoff_text`, `Handed`, `FailedCheck`, `MAX_HANDOFF_TOKENS`: what the session that picks up
  another's unfinished task is told: the agent and when it stopped, the files it edited and read
  (fewer named until the text fits 150 tokens), its last check when it failed, by its cleaned
  command in backquotes when it was kept (a command holding a backquote, or anything but plain
  text, is left out), else by its runner and test file, with the counts the check's output
  printed when they were read (`1 failed, 7 passed`), and its open warnings.

## Tests

```sh
cargo test -p cortex-coord
```

Every rule is tested on hand-built positions and links, the expected answers worked out by hand in
the tests' comments.
