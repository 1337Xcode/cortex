# Changelog

All notable changes to this project are documented in this file. The format is based on
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `cortex map`, which asks the daemon for the map's address, prints it and opens it in the browser
  (`--no-open` only prints it). The daemon serves the map on 127.0.0.1 alone, from the first ask
  until it stops, at `ui.port` (a new key; 0, the default, picks a free port). The address holds a
  random 64-digit token, and the daemon answers only requests that carry it, name the map by
  `127.0.0.1:<port>` or `localhost:<port>` and come from no other site's page; it sends no CORS
  header. It holds at most 64 connections at once and closes one after 30 s with no byte either way;
  when the daemon stops, the port is closed and so are the open connections. `cortex map` prints or
  opens only an address of exactly that form. A binary built after the map's interface was built
  embeds it; one built before serves a page saying it has no map. The map serves its page, its
  files, a health check and four read-only JSON endpoints read from the store: the files with their
  numbers and strongest links (`api/graph`), the first screen's summary (`api/summary`), a search
  of paths and symbols (`api/search?q=`) and one file's facts, history and what Cortex told agents
  about it (`api/file/<id>`). The guard's rules are tested one by one, and tests run the command
  against a daemon and send the map plain HTTP requests, the data endpoints included.
- `cargo xtask ui`, which installs the map's packages and builds the map for the daemon to embed;
  run it before a release build. `cargo xtask verify` now also runs the map's type check, linters,
  unit tests and build, and fails when the built map is over 1.5 MB gzipped; without the map's
  packages it says that it skipped them. `cargo xtask verify --full` runs the map's browser checks
  and checks its first load and frame rate at 100,000 files against their budgets (2 s, and 45
  frames per second at the 5th percentile) on macOS and Linux, and prints them on Windows.
- The map prepares its GPU programs while its data loads, starting all five links at once on a
  hidden canvas, so the driver compiles them side by side and the map's own canvas reuses them.
  On a Windows laptop (Intel Iris Xe, Direct3D 11, Chromium 158), at 100,000 files and in fresh
  browsers, the first load went from 1.83 to 1.98 s to 0.89 to 0.95 s over five runs each.
- Evaluation tasks for the history features, and the runner support they need. A task that says
  `history = true` is run in a clone of its testbed cut at the pinned commit (every branch and tag
  deleted, the remote removed, the reflogs emptied and unreachable objects pruned), so that
  `git log` and `git blame` work offline and nothing newer than the pin can be found; other tasks
  keep a copy of the files made a repository of one commit. `cortex-eval check --from` refuses a
  history task whose testbed folder is not a git repository holding the pin, and each task's check
  now runs with the run's own home folder. Nine tasks join the suite on serena and on a new
  testbed, hono: six where the reason behind the code matters and three controls where it does
  not. The runner's changes are tested on scripted repositories. Each task's check was run on a
  copy of its testbed and failed at the base and passed after a correct change, and the six
  history tasks' checks failed after a plausible wrong change; no task has been run with an agent
  yet.
- The experience hint, an experimental feature that is off by default (`history.experience`, the
  `experience` feature): a work order may end with one line of recorded facts about an earlier
  task or change like its prompt, once a task and within 80 tokens, shed after why-lines and
  before code. A past task is like the prompt when at least half of the files it changed are
  among those of the clusters of past tasks that match the prompt's words; a past change when
  its card scores 0.6 or more, as history seeds score it. Only work that ended before the task
  began is read. The line names the files the work changed and kept that are on disk now, and
  for a task one approach that did not hold: a test run whose same failing tests stayed failing
  until another file changed, else the files it put back. Task records now keep those two
  facts whether the feature is on or not: two failed runs are told apart by a short hash of
  the failing tests' names, taken only when the runner's counts vouch for the whole list, and
  the names are never stored. A store that fails the search costs the hint, not the work
  order. Tested on scripted stores; not run with a live agent yet.
- An origin note after an edit, part of the experimental why-lines and off by default with them
  (`history.why`): when the lines an edit of one hunk replaced came from a normal commit that
  names an issue or touched a test, the edit is told once a task, as a warning line, which
  commit wrote them (`The lines this edit replaced in src/money.py came from commit "Round
  half-even for tax" (fixes #612; test: tests/test_money.py), 2025-09-14.`), with no line
  number. Claude Code's report of an edit gives the hash of the file before it and the lines it
  replaced, which are looked up in the blame the daemon already keeps of that content; nothing is
  blamed for an edit. The note asks for nothing: it is never an open warning and is not counted
  with the warnings. Tested on scripted stores, and through the real binary on a scripted
  repository; not run with a live agent yet.
- History seeds, an experimental feature that is off by default (`history.seeds`, the
  `history_seeds` feature, and `history.seed_weight`, 0.4): a change a prompt names (`#482`,
  `!482`, or a commit id of seven hexadecimal digits or more) seeds the files it changed, and the
  three past changes whose words are most like the prompt's, at a score of 0.6 or more, seed
  theirs at `history.seed_weight` times the score, from the change cards the daemon keeps; each
  change's weight is spread over its files. Numbers that read as counts, ranks or colours name
  nothing. Seeded files are a kind of evidence of their own for the gate, which alone never
  clears it, a kept confidence model included. The bar was chosen on a replay of two Python
  projects' histories; tested on a scripted history, and its effect on work orders is not
  measured. `profile = "everything"` now turns `history.seeds` on too.
- Why-lines, an experimental feature that is off by default (`history.why`, the `why_lines`
  feature): under the code a work order shows, at most `history.max_reasons` lines (2) of
  `history.max_tokens` tokens (120) saying why those lines look as they do, from the commits
  that last changed them: the commit's subject and reason line, each quoted as the commit's, the
  pull request, the issues fixed, a test file touched and the day. Only a normal change that gives
  a reason line, a number or a test is a reason; a formatting, bot or mass change is looked
  through to the commit before it. The daemon blames the lines in the background and holds the
  commits' subjects and reason lines in memory only, so a span gets its reason from the next
  prompt on. A reason is given only for code shown as the file holds it, a file whose content git
  blamed: its line numbers are those of the code shown, and the reason is the commit that last
  changed those lines in that content.
  `cortex status` says what the history worker holds while it runs. Tested on scripted
  repositories, through the real binary included; not run with a live agent yet, and its effect
  on a task is not measured.
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
- `cortex init --agents` also writes Cortex's plugin for OpenCode (its 1.x plugin API, which
  init says OpenCode 2 does not run), Kilo Code, Amp and OpenClaw, in the folder each loads
  plugins from: the repository's or, with `--scope user`, the user's, in the configuration folder
  `XDG_CONFIG_HOME` names when it is set to an absolute folder, else `~/.config`; OpenClaw's always
  the user's `~/.openclaw/extensions/cortex/`, and on Windows, where its Gateway runs in WSL,
  nothing, with a line saying to set it up there. Cline gets a hook script per event in
  `.clinerules/hooks/` or in `Cline/Hooks/` of the documents folder the system names (on Windows
  as PowerShell gives it, so a folder moved to OneDrive counts; on Linux from `user-dirs.dirs`):
  `<Event>.ps1` with a UTF-8 byte order mark on Windows, a shell script with the executable bit
  elsewhere. A file of the user's with the same name is never written over; one an older build
  wrote is replaced. For OpenCode, Kilo Code and Cline, which load both folders, init refuses one
  while the other holds Cortex's files; Amp loads one plugin of a name, so both are written. Each
  of these agents also gets Cortex's MCP server in the user's JSON settings, beside the user's own
  servers and keys; a settings file holding comments, or a server named `cortex` that runs another
  program, is left as it is and init prints the entry to add instead (as `cortex-index` when the
  name is taken). Enabling OpenClaw's plugin and letting it see the conversation, and keeping
  Cline's Hooks setting on, are said and never done. For Hermes Agent and goose, whose
  configuration is YAML, init writes nothing and prints the lines to add: Hermes' hooks, which
  Hermes asks the user to approve, and MCP server, and goose's MCP extension (goose gets no hooks:
  Cortex has no adapter for its hook payloads yet). `cortex doctor` checks each plugin, hook script
  and MCP entry init wrote, in the folder the agent reads now, and says when the user turned one
  off (a hook script no longer executable, `"enabled": false` or `"disabled": true` on Cortex's
  MCP server), and looks for the printed lines in Hermes' and goose's configuration; `cortex
  uninstall` removes the files init made, gives settings files back byte for byte, keeps a plugin
  or script the user edited, and, from settings changed since, takes out Cortex's MCP server
  alone. These files are checked against fixtures and hand-written ones; the agents themselves
  have not run them yet.
- `cortex init --agents` also sets up the agents Cortex reaches through its MCP server alone:
  Zed, Continue, Warp, Windsurf's Cascade and Roo Code. Cortex's MCP server goes into the user's
  settings beside the user's own servers (Zed's `settings.json`, a `cortex.json` of its own in
  Continue's `~/.continue/mcpServers/`, Warp's `~/.warp/.mcp.json`, the `mcp_config.json` of the
  Devin configuration folder that Windsurf's Cascade reads); for Roo Code, whose settings file is
  not documented, and for settings holding comments, as Zed's often do, init prints the entry to
  add. A rules file of Cortex's own, `cortex.md`, tells the model to call `cortex_where` first
  when a task needs code it has not read: in a repository's `.continue/rules/`,
  `.windsurf/rules/` or `.roo/rules/`, left out of git through the exclude file git never
  commits, not `.gitignore`, or in `~/.continue/rules/` at the user's scope. A rules file of the
  user's at that path is never written over, and init makes no first file in `.roo/rules/` while
  a `.roorules` or `.clinerules` there would be hidden by it. Init never writes a file the user
  owns or other agents read, such as `AGENTS.md`, `.rules` or Windsurf's global rules: for Zed,
  Warp, and Windsurf and Roo Code at the user's scope, it prints the lines to add. For Aider,
  which has neither hooks nor MCP, it writes nothing and prints lines for the user's conventions
  file telling the model to suggest `cortex where`; `cortex doctor` says the steps are the user's
  once Aider is found. Doctor checks the MCP entries and rules files init wrote, and uninstall
  gives every file back byte for byte, the exclude file included. These files are checked against
  hand-written ones; the agents themselves have not run them yet.
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

- `privacy.store_prompt_excerpt` and `adopt.exclude` are read only from the user's own
  `~/.cortex/config.toml`. A repository can carry a `.cortex/config.toml` in its commits, and a
  clone or an archive could have turned on keeping the first 200 characters of each prompt
  without the user choosing it. A repository's file that sets either key is now ignored for it,
  `cortex config` and `cortex status` print a warning naming the file and the key, and `cortex
  config set` refuses these keys without `--user`. `cortex config` shows such a key's value as
  coming from the user's file or the default, never from the repository.
- A file the prompt names, by its path or by a rare name defined in it, now leads the work order
  with its code, and the folders of a path the prompt names are no longer search words: a prompt
  naming `src/app/helpers.py` no longer ranks every file under `src/app` above the one it names.
- A repository set up by an earlier build is served by hooks again once its daemon has started
  under this one and recorded it: run `cortex init` in it once.
- Cortex is licensed under the Apache License, Version 2.0, alone (see `LICENSE` and `NOTICE`).
  Commits before this change were offered under the MIT license or Apache-2.0, at the user's
  option, and stay available under those terms.
- `cortex mcp` serves only a repository set up on this machine and not turned off: the one
  `--root` names, which before was served whether it was set up or not, else that of the first
  folder the MCP client names as its roots that is in one (asked once a session), else that of the
  current folder. Anywhere else, as when an editor starts it from the home folder, each tool call
  is a tool error the agent passes on, saying Cortex is not set up there, is turned off there, or
  never serves that folder, and no daemon starts. The home folder, a folder holding it and a file
  system's root are never a repository: the daemon refuses such a root, and so do `cortex where`,
  `why`, `note`, `claims`, `consolidate`, `facts` and `import` at once, where they waited before
  for a daemon to start; `cortex init` and `cortex uninstall --purge` now refuse a file system's
  root too.

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

### Security

- Cortex no longer runs git in a repository whose own configuration includes another file
  (`include.path`, `includeIf`). Git reads what such a key names wherever it points, a network
  share included, and an included file can name a filter, a text conversion, a difference or
  monitor command, or a path that opens a connection. Before any other run the repository is
  asked, with includes off, whether it holds such a key, and if it does the work tree is not
  looked at, nothing is blamed or shown, and the history is not read with the git program;
  `cortex status` says the work tree is not read because the configuration includes another file.
