# Changelog

All notable changes to this project are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `cortex init --global` sets Cortex up once for the user: every agent found gets Cortex's
  hooks in its user settings, beside the user's own, the holdout question is kept in
  `~/.cortex/config.toml`, and adoption is turned on (`adopt.enabled`, which `cortex config`
  shows and sets). It refuses while a repository `cortex init` set up holds Cortex's hooks in
  its own settings, so that no agent runs Cortex twice there. From then on, the first hook from
  a git repository below the home folder that is not set up starts a daemon that sets it up on
  its own, all or nothing: `.cortex/` and its lines in the exclude file git keeps, never a
  `.gitignore`. The first session start or prompt after that tells the user once, in the
  message the agent shows its user and never to the model, that Cortex set the repository up
  and that `cortex off` turns it off; the tests check this with Claude Code's and Codex's
  payloads. The home folder, the temporary folder, the user's own Cortex folder, folders
  outside git and folders named by the new `adopt.exclude` setting are never set up and keep
  nothing of Cortex's; a refusal is remembered so that hooks stop asking, and `cortex status`
  and `cortex doctor` show it, while a set-up that fails is logged and tried again later. The
  hook itself only looks at file metadata and starts the daemon in the background; it reads no
  configuration, runs no git, and a `cortex doctor` probe offers nothing. `cortex uninstall
  --global` gives each agent's file back byte for byte and turns adoption off. `cortex off`
  turns Cortex off in one repository, and `cortex on` turns it on again.
- Hooks serve only a repository the user's own folder records as set up on this machine
  (`~/.cortex/repositories/`, written when its daemon starts, removed by `cortex uninstall`): a
  `.cortex` folder a repository carries in its files no longer makes hooks serve it.
  These were run through the binary with recorded payloads; a live agent session has not run
  them yet.
- `cortex init --agents` also writes Cortex's hooks for Cursor, GitHub Copilot, Antigravity,
  Kiro, TRAE, Crush (in `crush.json`, the JSON form Crush still reads beside its newer `crushrc`)
  and OpenHands, each in its own hooks file and shape as its documentation gives it, beside the
  hooks the user already has there, keeping the keys the user added to Cortex's hooks, such as a
  switch turning them off; OpenHands' events are spelled and placed as the user's own are.
  Copilot's and Kiro's go in a file of Cortex's own in the repository, which `.gitignore` then
  gets, or in the user's hooks folder with `--scope user`, and init refuses one scope while the
  other holds Cortex's hooks, since the agent would run both. What the user must turn on
  themselves (TRAE's hooks switch, Copilot's trust of the folder) is said and never written.
  `cortex doctor` checks that every agent init set up for the repository, and uninstall has not
  undone, still holds Cortex's hook for each of its events, and names the init that puts them
  back, or the user's own switch when the user turned them off. `cortex uninstall` gives those
  files back as they were, removes those init made, and removes exactly the folders init made for
  them, once empty. These files are checked against hand-written ones; the agents themselves have
  not run them yet.
- `cortex` run with no arguments, its standard input not a terminal (as an agent runs a hook),
  prints nothing and exits 0, so that an agent that imports Claude Code's settings and runs
  Cortex's hook program without its arguments never reads a usage error as a refusal. A payload
  given to Cortex's Claude Code hook that carries a field only Cursor, GitHub Copilot or TRAE
  sends is passed over, so that no session is recorded twice.
- Linked working trees (`git worktree add`): an agent working in one reaches the repository's
  daemon, store and what Cortex learned, never a second setup of its own; its work orders,
  warnings and hints show and check its own files' lines, its uncommitted changes are its own,
  positions and coordination notices name the working tree and branch, and `cortex init` there
  registers the agents' hooks in it, makes no `.cortex` there, and writes its ignore lines to the
  exclude file git keeps for every working tree of the repository instead of a branch's
  `.gitignore`; `cortex uninstall` undoes both.
- `cortex init` sets a repository up for Claude Code: it finds the repository with git, checks
  Claude Code's version, asks two questions once (the holdout, team sharing),
  registers Cortex's hooks in `.claude/settings.local.json` (or `~/.claude/settings.json` with
  `--scope user`) beside the user's own, backs up every file it changes with a hashed manifest,
  starts the repository's daemon, waits for its first index and ends with `cortex doctor`.
- `cortex uninstall` restores every file init changed from its manifest and stops the daemon;
  `--purge` also deletes the repository's `.cortex/` folder, except `.cortex/shared/`, and its
  manifests and daemon log in the user's folder.
- `cortex doctor` checks that Claude Code will run Cortex's hooks for the repository and that the
  daemon answers each hook within its latency budget, and prints the fix for each failure.
- `cortex status` reports the daemon (process, version, uptime, queue, hook latency percentiles,
  recent errors), the index and scale mode, the git history mined, the work tree read, the graph,
  the gate's bar and what the self-check last found.
- One daemon per repository keeps an index current as files change. Files are discovered with
  secret, binary and generated files left out; symbols, references and imports are parsed for
  Python, JavaScript, TypeScript and TSX, Go, Rust, Java, Ruby, C, C++, C#, PHP, Kotlin, Swift,
  Scala and Bash, plus Markdown headings, SQL definitions and top-level JSON, YAML and TOML keys;
  imports are resolved to files, and a SCIP index can supply references at full confidence. Large
  repositories are indexed in a lighter mode. On Windows the daemon holds none of the handles of
  what started it, so an agent or script reading a hook's or command's output through a pipe sees
  it end with that process.
- Through Claude Code's hooks, the daemon records what agents read, edit and run, as sessions,
  tasks and events, never the content of files, prompts or tool output.
- Co-change mining reads the first-parent git history (with `gix`, or the `git` program when `gix`
  cannot) into weighted links between files that change together.
- Learning: when a task ends, Cortex judges what became of it, earns interest for the code it
  used, draws links between code used together and groups similar tasks.
- Retrieval: from a prompt, Cortex seeds the graph from the words and paths it names, the
  session's own work, the uncommitted files and similar past tasks, spreads relevance over files
  first and then over the best files' symbols. It gives a work order (the files and line ranges
  to start from, each with its reason, checked against the file on disk when it is written) only
  when the evidence agrees, and nothing otherwise. Each session starts with
  a short, byte-stable primer.
- The gate's self-check raises the bar when the work orders given go unused.
- `cortex where "<question>"` prints what the engine finds for a question, or why it finds
  nothing.
- `cortex receipts` shows each session's prompt-cache health: the share of its input read from
  the cache, the share of its estimated cost spent writing to it (when every response is priced),
  and how often a conversation (the session's or a subagent's) lost its cache, by the likeliest
  cause (a model switch, a compaction, idling past the cache's lifetime, which is an hour when
  every write was for an hour and five minutes otherwise, or text Cortex gave just before),
  the losses with no known cause listed by turn and time. A loss is a response that wrote over
  half its cached input anew while reading under half of what the response before it read.
- `cortex receipts` shows each session's and task's token usage and estimated cost, read from
  Claude Code's transcripts, the tokens Cortex injected, the share of its work orders that were
  used, and, once each arm has at least 30 ended tasks, the difference between the tasks Cortex
  held out and the rest, each difference with a 95% bootstrap interval.
- An evaluation suite (`eval/`): tasks that Claude Code works through in fresh copies of a pinned
  testbed, with success checks and a report of precision and usage, each share with a bootstrap
  interval that resamples whole tasks; `cortex-eval report` rebuilds a results folder's summary
  from its runs. The first measured results are in `eval/results/2026-11-05-c856abc/`.
- `cortex version` prints the binary's name and version.
- After Claude Code edits a file, it is told what the edit seems to leave undone: when a
  function's signature changed, the calls to it to check in other files the task has neither read
  nor edited (how many, and five listed by path and line, each line checked against the file on
  disk); the file's own
  tests (those named after it), once a task; and a file that usually changes with it, and seldom
  without it, in the repository's history or in earlier tasks, when the task has not touched it,
  each such file once a task. At most three warnings an edit, call sites first, then tests, then
  files that change with it, the strongest first within each, within 250 tokens;
  `warnings.max_per_edit`, `warnings.cochange_min_confidence`, `warnings.cochange_min_count`,
  `warnings.cochange_min_link_weight` and `warnings.cochange_min_link_tasks` set the limits.
- Before Claude Code reads a file, it is told when the session already read those lines since its
  context was last compacted and the file has not changed since, or, in a file over 600 lines,
  which parts of it the task's work order pointed at, each line checked against the file on disk,
  within 120 tokens. A tiny repository gets no hint.
- When a task ends, each warning its edits got is marked resolved if the agent then read or edited
  the file it named. `cortex status` and `cortex receipts` show the share of warnings followed up
  this way.
- After Claude Code compacts a session's context, the start that follows is told what the task in
  progress was doing: the files it edited, the files it was warned of and has not read or edited
  since, and the start of its prompt when `privacy.store_prompt_excerpt` kept it, within 500
  tokens, once for each compaction.
- When a session stops before finishing its task (cut off, or answered with no check that
  passed), the next session's first prompt is told once what was left: the agent and when, the
  files it edited and read, its last check when it failed, with the counts the check's output
  printed (`1 failed, 7 passed`, read in memory; the output is never stored), and how many
  warnings about its edits were still open. The failed check is named by its command, cleaned:
  only a known runner's own words are kept, by the program's name alone, without environment
  assignments, and every argument a credential detector matches (token shapes, JWTs, `Bearer`
  values, values of credential options and keys, URL credentials, long high-entropy strings) is
  replaced by `<redacted>`; when the command cannot be cleaned with certainty only the runner and
  its test file are kept. The command is cleared when the handoff is taken, and after a day.
- `compaction.keep_list = true`: just before Claude Code compacts a session's context, its
  `PreCompact` hook prints, as instructions for the compaction's summary, what the session's open
  task has done: the files it edited, the files it was warned of and has not read or edited since,
  the project facts it was given that still hold, and the start of its prompt when the user's
  settings keep one: the lines the recap after a compaction gives, at most 600 characters. Nothing
  is printed in a tiny repository, outside a task or for a held-out task. Codex ignores a
  compaction hook's output and is given nothing. Off by default until measured.
- `hints.narrow_tests = true`: after a run of a whole test suite (`pytest` with no path, `cargo
  test` with no filter, `go test ./...`, `npx jest`), the task is told once the test files of the
  files it edited, each file's own tests first, at most three, and the command that runs only them
  as the suite was launched, its own options repeated (`python -m pytest -q tests/test_ops.py`)
  except those that report on the run or check its coverage, within 80 tokens. The command stands beside the files it runs; a test file it cannot run (a unit
  test inside a source file, another package's) is named after it. A run whose options change
  where modules import from (unittest's `discover -s`) is told nothing. Off by default until
  measured.
- `hints.redundant_commands = true`: just before a test or build command runs, when the same
  command line ran in the same working folder earlier in the session and no source change was
  heard of from the moment that run started (no edit a hook reported, no shell command that may
  have written a file, and no change the file watcher saw to the repository files it watches;
  files the watcher leaves out, such as those `.gitignore` names, are not seen), the agent is told
  what that run gave: its exit code, and the failing tests its output named, at most three and
  then how many more (``[cortex] `pytest tests/test_ops.py` ran earlier in this session with no
  source change since: exit 1, failing: test_divide_by_zero.``). A shell command counts as a
  possible change unless every program it runs is a check or one known to write nothing (`ls`,
  `cat`, `grep`, `git status` and a few more) with no output redirected to a file. The command
  still runs. At most two notes a task, within 60 tokens; nothing in a tiny repository, outside a
  task or for a held-out task. The results are kept in the daemon's memory only, by a hash of the
  line and the folder, never stored. Claude Code's `PreToolUse` hook now also runs before `Bash`
  and `PowerShell` calls, so every shell command starts the hook; Codex's already ran before every
  shell command, and Codex gets the note too. Off by default until measured.
- `coordination.contracts = true`: when another session changes the signature of a definition
  that a file a session holds calls, that session's notice says what the signature was and is,
  and the lines of its file that call it (``[cortex] Coordination: Codex changed add in
  calc/ops.py from `(a, b)` to `(a, b, c=0)`. You read main.py, which calls it at line 3.``), at
  most three definitions and three lines each, the rest counted. The signature before is the
  header the index read before the change, as its stored hash vouches, taken from the
  daemon's memory of what it parsed or else from the file as HEAD's commit holds it; the
  signature after and the calling lines are checked against the files on disk when the notice
  is given, one notice a hook. A notice the files no longer bear out, over the token cap, or
  whose signatures cannot be said (one over 120 characters is cut after a parameter; one
  holding a backquote or a control character is not said) names the definition alone, as it
  does with the key off. Signatures are kept in the daemon's memory only, and the store keeps
  the notice naming the definition alone. Off by default until measured.
- The file watcher reports a notice that file events were lost (a rescan) as a change to
  everything, as it does an error, rather than dropping it; and it no longer reports a folder
  that is still there whose times or attributes changed (Windows reports one when an entry in it
  was touched), since its entries' own events say what changed in it.
- `hints.runtime_grounding = true` gives a failed check the repository lines its stack ran
  through, beside its output: when a test or build command the agent ran fails (Claude Code's
  failure hook, Codex's shell result with a non-zero exit), its output is read in memory by the
  parser of its runner (pytest, unittest, cargo test and rustc, go test, jest and vitest, tsc,
  Node stack traces, Maven and Gradle), each stack frame is mapped to a file the index holds as it
  is on disk, and up to three of them, the innermost first, are named with their code line and
  the definition holding it, within 120 tokens. Nothing is given when every code line is in the
  output already and the task has read every file named, or when no frame is in the repository
  (an import or environment error). The session's next prompt also seeds its retrieval from
  those frames, as strongly as an edit where the prompt or the session points at them too, as a
  read where nothing else does; a passing run of the same runner, or half an hour, drops them.
  The output is never stored. Off by default until measured.
- `strict_mode = true` gives Claude Code a reason to go on when it stops while files calling a
  signature its task changed were not read or edited since the warning, once a task. Off by
  default, when a stop is given nothing. The hook and the daemon now speak protocol 5.1: a hook and
  a daemon of different Cortex versions are replaced as before.
- A long prompt, such as a pasted specification, is looked up by what it writes most: a name it
  writes in passing no longer counts as naming its file, and only its 256 most written words and 32
  most written names are looked up.
- A work order over its token cap now cuts its last code to its first lines, saying which lines
  are not shown, rather than dropping the code whole, and shows a method under its class's first
  line.
- A work order no longer shows code the session's agent was shown by an earlier work order, or
  read, since its context was last compacted, while the file is unchanged.
- A slash command, a shell escape (`!`) or a prompt of fewer than four characters such as "ok" asks
  for no work order: the hook sends the prompt to be recorded, and the daemon retrieves nothing.
- Every text Cortex gives a task is recorded with its tokens and counted in `cortex receipts`: work
  orders as before, and now an edit's warnings, a read's hint, the recap after a compaction and
  strict mode's reason. A held-out task is given none of them, so the comparison between held-out
  tasks and the rest holds out everything Cortex would say to a task.
- `cortex config` prints every configuration key with its value in effect and where it came
  from (`default`, `profile`, `user`, `repository`), as text or with `--json`. `cortex config
  set <key> <value> [--user]` writes one key into the repository's `.cortex/config.toml` or the
  user's `~/.cortex/config.toml`: an unknown key is refused naming the nearest known one, a
  value is checked as loading checks it, only the key's own line changes (comments and every
  other line kept), and what the file held before Cortex first changed it is kept as
  `config.toml.cortex-backup`. `cortex init` adds its holdout answer through the same
  editor, so an answer now goes under a `[holdout]` table the file already has.
- The `profile` key chooses the feature switches' defaults: `standard` (the defaults), `lean`
  (every hint and the keep-list off) or `everything` (each on); any key a file sets overrides
  it.
- `cortex features` lists what Cortex gives agents, whether the settings let each reach a
  task, its key and the comparison that decides it, with no effect shown until one is measured;
  `cortex features on|off <name>` sets its key.
- `cargo xtask verify` runs formatting, the crate dependency-direction rules, the shared lint
  settings in every crate, the invariant tags, clippy, the tests and the dependency licence and
  advisory checks; `--full` also runs the benchmarks against their budgets.

### Changed

- A file the prompt names, by its path or by a rare name defined in it, now leads the work order
  with its code, and the folders of a path the prompt names are no longer search words: a prompt
  naming `src/app/helpers.py` no longer ranks every file under `src/app` above the one it names.
- A repository set up by an earlier build is served by hooks again once its daemon has started
  under this one and recorded it: run `cortex init` in it once.
- Cortex is licensed under the Apache License, Version 2.0, alone (see `LICENSE` and `NOTICE`).
  Commits before this change were offered under the MIT license or Apache-2.0, at the user's
  option, and stay available under those terms.

### Fixed

- The lines `cortex init` adds to a `.gitignore` no longer count as uncommitted work: a
  prompt right after install is not taken to continue a change of the `.gitignore`.
- A prompt's retrieval no longer walks every symbol node of the repository: the store's lookups of
  a named symbol, of a listed file's symbols and of the files of nodes start from their own index,
  and the co-change pairs of a file are found by either of its sides through an index (a schema
  migration adds it). On a 10,000-file test repository a retrieval in the daemon went from 136.9 ms
  to 33.3 ms at the 95th percentile on Windows.
- While a large index runs on a busy machine, hook events reach the store within a second again:
  the index writes references and edges in transactions cut every quarter second rather than every
  4,096 rows, reads the whole repository's names in steps, and walks the repository on a thread of
  its own while it records the events waiting.
