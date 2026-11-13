# Cortex map UI

The map is a web page that shows a repository as a graph: directories are hubs, files sit on
rings around them, and the links Cortex has learned (files changed together, files used together,
files named in work orders) are drawn between files. The Cortex daemon serves the built page on
127.0.0.1, and the page reads its data from the daemon's `api/` endpoints with the access token
that is part of the page's own URL. It asks nothing of any other origin.

This folder holds the page's source. It is written in strict TypeScript with React, Vite,
deck.gl, d3-hierarchy and Zustand, with shadcn/ui components on Radix and Tailwind CSS.

## What the page shows

- A strip at the top that stays one line: the most useful sentences that fit whole, in full or in
  a short form (the strongest pair of files changed together, the folder agents worked on most in
  the last week, how many files agents have read, how many files and folders are indexed). Nothing
  is cut inside a word. A repository Cortex has learned nothing about says so, and the map shows
  its structure alone.
- The overview panel on the left, in the order of how often a person asks its questions: the agent
  sessions running in the repository now; the activity; every summary sentence, with when the map
  read its numbers; the receipts; the strongest pairs of files changed together (by the link's
  strength times the logarithm of one plus its commits, as Cortex ranks a file's partners and the
  summary its strongest pair, so a pair that often changed together comes before one of two
  commits, which the section says; each row shows its commits and says "changed with mod.rs in 12
  of 21 commits" on hover); the busiest files (by the tokens attributed to them, or by commits
  before agents worked on any); the files that need attention (a suspect fact); the warnings; and
  the daemon's health. Each list shows five rows and then "Show N more", and each file row is a
  button that flies to its file and selects it. Each section collapses to its title and count: the
  full count, or "top 25 of 2,194" when a list keeps only its strongest (a list of links says under
  its title that it counts only the links the map keeps: each file's 8 strongest of each kind). The live sessions and the activity open while a session runs and are
  collapsed while none does (the live sessions then say "No agent session is running in this
  repository"); once the person opens or closes the activity it keeps their choice. The receipts,
  the warnings and the health are closed at first and asked for when opened. On a narrow screen the overview is a bottom sheet
  opened from the top bar.
- The activity: what agents' hooks reported lately, newest first, among the 100 newest events Cortex
  recorded (edits seen outside any agent left out), each a sentence led by the agent's icon and the
  file's ("Claude Code 3f2a… edited book.rs") with how long ago it was ("2 min ago"). "Ago" counts
  back from when the page read the list, by this computer's clock, and the section says so; each
  row gives its exact time in UTC, the reporting hook's own clock, on hover and to screen readers.
  The daemon names a file by its id and its path, and ids move when files are added, so a row is a
  button that flies to its file (and says its time on keyboard focus too) only where the map on
  screen holds that path; a file it does not hold yet is named by its whole path, as text, with
  "not on the map yet" ("not on the map" for one the map does not list at all), and the page reads
  the map again. Each row is two lines that wrap, never cut. A failure says what failed and offers to
  try again.
- The warnings Cortex gave agents after their edits, open first and then resolved, among the 100
  newest: each says what it said by its kind ("book.rs usually changes with mod.rs", "orders.rs is
  a test of book.rs", "app.ts calls a symbol of book.rs whose signature changed"), then whether it is
  open, followed (the named file was read or edited after it) or not followed, its strength and its
  date; its whole sentence, with what the strength measures for that kind, is said on hover and
  focus. A strength is never rounded to a false 0 or 1 ("under 0.01"); a test or call-site warning
  that named several files keeps the strongest of them, and says so. A row whose named file the map
  on screen holds, at the path the daemon gave, flies to it; any other file is named by its path, as
  the activity does.
- The daemon's health, the last section, closed at first (a person needs it when something looks
  wrong, and a header chip would take room the search box and the strip need on a narrow screen):
  the version and whether the first reading is done, how long the daemon has run and what it
  dropped or failed to record, each hook's handling times (median, 95th and 99th percentile) in a
  small table that says they are the daemon's own handling of each request, not the hook process's
  whole run, its newest errors in its own words, the history miner's state, the graph's size,
  upkeep's last run and whether the history worker is on, off or refused; with when it was read and
  a button to read it again. Its count is the errors the daemon kept. A report the page cannot read
  says so with its cause, and the rest of the answer still counts: the empty map still says whether
  Cortex is reading the repository.
- While agents work, a live frame that brings a session's new position asks for a quiet refresh: two
  seconds later, and at most once every ten seconds, the page reads the map, the summary, the
  selected file's panel and the activity, warnings and health once their sections have loaded them
  (the receipts are read once), without a loading state. The selected and hovered files are found
  again by their paths, so the file panel stays open on its file, its sections shown while they are
  read again, when a file added before it moves its place. An answer that names a file the map on
  screen holds elsewhere, or not yet, asks for the same refresh, once per map. A map of more than
  20,000 files does not refresh by itself; it says when its numbers were read and offers "Read
  again".
- The receipts, a section of the overview under "At a glance": what agents' work in the repository
  cost and whether Cortex helped, in the words `cortex receipts` prints. The section is closed at
  first and the page asks the daemon for `api/receipts` only when it is opened. It says, in order:
  - The headline. Cortex holds some tasks out and compares them with the rest only once each side
    has 30 tasks that ended. Before that the headline says "Not enough data yet" with the counts so
    far ("29 of 30 held-out and 29 of 30 treated tasks so far"), and the section shows no
    difference, share or saving anywhere. Once there are enough, it lists each measure as the CLI
    does: the success rate and, with 30 successful tasks a side whose usage was read, the median
    input tokens, uncached input tokens, turns and tool calls per successful task, each with its
    value with Cortex and held out, the difference (with Cortex minus held out) and its 95%
    bootstrap interval, and the sample sizes. A measure whose interval holds 0 says that it is not
    a measured difference.
  - How precise the work orders Cortex gave were (of those labelled, how many were used) and how
    often the files warnings named were then read or edited, with the counts.
  - A chart of tokens and turns per task: two small plots on the same tasks, input tokens above and
    turns below, each on its own scale from 0, one mark per task, oldest on the left, for the
    newest 60 tasks of the sessions shown whose agent's usage Cortex reads (it says how many it
    leaves out). The task under the pointer, or the one picked with the arrow keys once the chart
    has the focus, is explained below the plots in one sentence with its numbers, the line the CLI
    prints for it.
  - Each session of the last 7 days (the newest 100), five at a time, newest first: its agent and
    state, when it started, its total, and the tokens Cortex injected (its primer's among them)
    against the session's input tokens. A session whose agent's transcripts Cortex does not read
    says its usage is not shown.
  - The price table every cost is estimated from: the user's price file, the table Cortex ships
    when there is none, or that no cost is estimated and why. Costs are always called estimates.

  With no session in the window it says so and how to get receipts (`cortex init` installs the
  hooks); a failed load says what failed and offers to try again. The page checks every field of
  the answer and the sums that tie them together (a session's turns are its tasks' and the rest, a
  comparison only with 30 tasks a side), and refuses an answer with more than 100 sessions, 2,000
  tasks in a session or 5,000 cache losses in a session.

- The graph. Shape says what a mark is and colour confirms it: folders are squares and files are
  hollow circles sized by their line count; a file with a suspect fact has a second ring (amber), a
  file an agent read or edited in the last 24 hours a filled centre (green), and a file no agent
  has read a dimmed, thinner outline. Learned links are drawn in one accent colour, their kind told
  by the line pattern (solid: changed together; dashed: used together; dotted: named in a work
  order) and their weight by their opacity. Only links at or above the strength set in the legend
  are drawn. Where many links are near the screen only the strongest few of each kind stay strong,
  and on the overview, until the person zooms in or picks a file, only the strongest few are drawn
  at all. The legend shows each shape with its count and one line on what it means.
- Views. A switch beside the zoom (and the keys `1` to `5`) colours the same graph to answer one
  question at a time; shapes keep their meaning in every view and only colours change. Default is
  the look above. Fog lights the files agents have read and leaves the rest dim and thin, so a
  repository no agent has worked in is all fog, on purpose. Heat steps the tokens attributed to
  each file (input and output tokens, cache reads included) through four shades of the accent on a
  log scale from the fewest to the most any file has. Coupling steps how many links the map keeps
  for each file, of any kind and strength (each file's 8 strongest of each kind, and those its
  neighbours keep to it, not every link Cortex learned), the same way. Recent lights the files an
  agent read or edited in the 24 hours before the map last read its data. In
  these four views a lit file is also filled with a share of its colour, and a folder drawn whole
  takes the colour of its strongest file. The legend names the active view, says what its scale
  means with its numbers, and counts the files in each step.
- Live sessions. While an agent session runs in the repository, a diamond in its agent's colour
  (one colour each for Claude Code, Codex, Cursor and Gemini, a shared grey for the rest) sits
  around the file it worked on last, named with the agent and a short session id, and a line runs
  back through the last ten files it worked on, fainter with each step. Zoomed out, a file drawn
  inside a folder is marked at the folder. Under the pointer a marker says what the session is doing
  ("Claude Code session 3f2a… is editing book.rs; before that it read mod.rs and payment.rs").
  The daemon pushes the whole live state over a WebSocket at the page's own address, and the page
  sends nothing back; a frame never moves the camera or the layout. A frame that breaks the
  contract is dropped with a console warning. When the connection is lost the page says "Live view paused,
  reconnecting", hides the markers rather than show them stale, and tries again after 1 s, doubling
  up to 30 s.
- Fact badges. Zoomed in far enough to read file names, a file with facts carries a small dot on
  its rim, amber when one of them is suspect (that file also keeps its second ring); the count is
  said on hover.
- Level of detail. Zoomed out, a folder whose rings would be too small to read draws whole, as a
  translucent disc around its square, named with its file count ("orders · 25 files") once it is
  large enough to carry a name; links of its files are drawn to it. While rings are only a few
  pixels apart, each folder draws its own files as one disc. A repository of 100,000 files opens on
  an overview of its top folders.
- Labels where they can be read: folder names wherever they fit, file names once zoomed in, and
  always the hovered or selected file (even in a dense ring) and its linked files. Labels never
  overlap each other or the page's controls.
- Every mark and link says what it is under the pointer, with its number ("payment.rs: 220 lines,
  read 9 times by agents, changed in 13 commits"; "book.rs changed with mod.rs in 12 of 21
  commits"; "book.rs and app.ts were used together by agents in 3 tasks"; "named together in 2
  work orders"): each kind of link by the count its strength was made from, and the legend says
  what strength means for each kind. A generated or vendored file, whose commits Cortex does not
  count, says so instead of a 0. Hovering a file also highlights it and its linked files and dims everything else;
  clicking selects it and opens its panel. Clicking a folder zooms in until it opens. Screen readers
  hear the selection as it changes, not every file the pointer passes.
- Names that would read the same say where they are: two files called `mod.rs` among the links
  show as "mod.rs in orders" and "mod.rs in billing".
- The panel of the selected file: the breadcrumb of its folders, each a button that frames that
  folder whole and open (the keyboard's way into a folder drawn collapsed); its name and path; its
  lines, agent reads, commits and the tokens attributed to it (the tokens of agent turns whose last
  read or edit was this file, cache reads included), in tabular figures. Then sections that collapse
  to their title and count. Its strongest links of each kind (strongest first as Cortex ranks them,
  each with its commits, tasks or work orders; "top 8 of the 12 the map keeps" when it has more) and the facts
  anchored to it are open, as they answer the questions asked most (what usually changes with this
  file, and which rules apply to it). Each fact offers the person's word on it as a native button:
  a candidate "Confirm", an active fact "Retire", a suspect fact "Confirm again" and "Retire".
  Retiring asks first, in the row, with "Keep it" focused (Escape keeps the fact); while the word
  is on its way its button says "Confirming…" or "Retiring…" and the others wait; then the section
  says what came of it in words (the daemon's own words when it refused; an answer the page cannot
  read, or none at all, says the word may have been written and to reload to see), and after a word
  that was or may have been written the page reads the panel and the map again. The word goes to
  the daemon as a POST with no body, in `fetch`'s default mode; the daemon serves the page with the
  `same-origin` referrer policy, under which a same-origin POST carries the page's own `Origin`,
  and its writer carries the word out. The commits behind
  its current lines (the ten newest, "the 10 newest of 34", or, with the history worker off, that
  Cortex reads them only when `history.why` or `history.seeds` is on and how to turn it on), each
  time Cortex named it to an agent (the latest 20), with whether the agent then read or edited it,
  and its symbols, open when asked. The symbols are the first 500 the index holds for the file, in
  line order, nested ones indented; those holding lines the latest work orders that listed the file
  selected are highlighted with how many orders chose them, above the selected spans themselves.
  With an editor link each symbol opens the file at its line, or at line 1 when the editor address
  does not show where its line goes, which the section then says. Its commits and tokens say what
  they count on hover and keyboard focus: the commits are only those of the history Cortex reads
  (its most recent commits, within an age, leaving out commits that changed too many files, each
  limit as the daemon reports it), so 0 never reads as "never committed". "Open in editor" opens
  the file in the editor the daemon names; the page shows the link only when its scheme is not a
  web, script, data or file address.
- Long text (paths, names, commit subjects) is cut at a path or word boundary with an ellipsis and
  shown whole on hover and on keyboard focus; nothing scrolls sideways.
- Icons beside names: an agent's mark (its product's, or its maker's when the product has none)
  before each live session, before each session of the receipts and before each agent the legend
  lists as working now, with its diamond in its colour and its sessions counted; a file's language
  or format mark (by its whole name for a tool's own files, such as `Cargo.toml` or `Dockerfile`,
  else by its extension) before its name in the overview's lists, the search results, the linked
  files and the selected file's title. A row that names two files shows the mark both share, or
  none. An agent or a kind of file with no mark drawn in one colour has no icon, and nothing stands
  in for one; in a list its row keeps an empty space, so the names still line up. An icon only
  repeats the name beside it, so screen readers skip it.
- On first open, one sentence says what the picture is. Once dismissed it stays dismissed (the
  browser's local storage remembers it), and the legend opens it again.
- Every state says what it means and what to do next: loading, a failure with its cause, nothing
  found by a search, and an empty map. While the map is empty the page asks the daemon's health
  every 5 seconds: while Cortex is still reading the repository it says so, and once it has
  finished the page asks for the map again and shows it by itself; a repository Cortex finished
  reading with no files says that plainly.
- Text from the repository is shown as text. A path holding an invisible or bidirectional
  formatting character is refused; in commit subjects, refs and facts such characters show as
  `�`. If drawing the page fails, it shows the error instead of a blank page.

## Keyboard

| Key                                 | Does                                                               |
| ----------------------------------- | ------------------------------------------------------------------ |
| `/`                                 | Focus the search box                                               |
| Enter (in search)                   | Fly to the first result and select it                              |
| Up and Down                         | Move through the search results                                    |
| `Z`                                 | Fit the whole graph                                                |
| `C`                                 | Centre on the selected file, or on the graph                       |
| `+` and `-`                         | Zoom in and out by a factor of 1.3                                 |
| `L`                                 | Show or hide the legend                                            |
| `1` to `5`                          | Colour the map by the Default, Fog, Heat, Coupling or Recent view  |
| Arrow keys                          | Pan, unless a search result has the focus                          |
| Escape                              | Close the search results, or close the side panel and return focus |
| Enter on a folder of the breadcrumb | Open that folder, filling the view, and keep the file selected     |

Every control is a native button or input and shows a focus ring. Motion lasts 150 ms and is off
when the system asks for reduced motion. Light and dark themes follow the system setting.

## Design system

Every colour, size, line height, stroke and timing is a token in `src/theme/tokens.ts`;
components and stylesheets hold none of their own (an ESLint rule checks the components and a
unit test scans the stylesheets). Sizes sit on one 4 px grid: spacing 4, 8, 12, 20, 32 and 52 px,
growing by about the golden ratio from 8 on, and type at 12, 13, 16 and 21 px on line heights of
16, 20, 24 and 32 px. Colours are written in OKLCH: twelve neutral steps per theme, one violet
accent for learned links and the selection, and green and amber for recent and suspect files at
the accent's own lightness and chroma, 35 degrees either side of its complement. Every text and
mark pair meets WCAG AA in both themes (4.5:1 for text, 3:1 for marks), checked by a unit test,
and a browser check compares every token as Chromium paints it with the channels the canvas draws.

Every word is set in Geist, and paths, line numbers and numbers in tables in Geist Mono (both
under the SIL Open Font License 1.1, from `@fontsource-variable/geist` and
`@fontsource-variable/geist-mono`). Only their Latin variable-weight files are bundled, 52,584
bytes gzipped together, served from the map's own address. The canvas builds its first label
atlas only once both fonts have loaded, so no label is drawn in a fallback font. The fonts'
copyright notice and licence are in `public/LICENSE-fonts.txt`, which the build copies into the
page, so every Cortex binary carries it beside the fonts and the map serves it.

The icons come from `@thesvg/react` (MIT), pinned to one version. Each is imported by its own path
(an ESLint rule refuses the package's index, which holds every icon it has), so the page carries
only the icons in the two tables of `src/icons/`: one for agents, one for files. Only an icon drawn
in one colour is used, in the muted text colour, so the tokens set its colour and its contrast in
both themes; a unit test renders every icon in the tables and checks it holds no colour of its own
and is the icon its name says. Brand marks are the trademarks of their owners and are shown only to
name their products. The package's published files carry no licence text, so the MIT licence of
the repository it is published from, at the tag of the version used, is in
`public/LICENSE-icons.txt`, which the build copies into the page beside the fonts' licence.

The controls and panels use shadcn/ui components kept as source in `src/components/ui/`, on Radix
primitives and Tailwind CSS. Tailwind's own scale is left out: the theme in `src/index.css` maps
only the token names (`p-sm`, `h-control`, `text-small`, `bg-primary`) onto the custom properties
`src/theme/applyTheme.ts` writes from the tokens.

## What is here

| Path              | Holds                                                                                       |
| ----------------- | ------------------------------------------------------------------------------------------- |
| `src/api/`        | One client per endpoint, and the live view's socket, each with a guard against the contract |
| `src/state/`      | The Zustand store: load states, the selected and hovered file, the link strength threshold  |
| `src/layout/`     | The deterministic radial layout and the level-of-detail helper, both pure functions         |
| `src/map/`        | The deck.gl canvas: encoders from the layout's typed arrays, label placement, the camera    |
| `src/views/`      | The views, one file each and one registry line: each sorts files into tones with a legend   |
| `src/controls/`   | Fit, Center and zoom, the legend, the search box and the shortcut table                     |
| `src/panels/`     | The summary strip, the overview panel and the file panel, with their sections and lists     |
| `src/components/` | The shadcn/ui components (button, tooltip, collapsible) and text cut to its box             |
| `src/fonts/`      | Loading the two bundled fonts, which the canvas waits for                                   |
| `src/icons/`      | The agents' and files' icons: one table each, and the box an icon is drawn in               |
| `src/intro/`      | The sentence shown on first open                                                            |
| `src/theme/`      | Design tokens for both themes (every colour, size, opacity and timing) and their CSS        |
| `src/text/`       | The map's words, cutting text at a boundary, measuring it, formatting numbers and dates     |
| `e2e/`            | Playwright checks driven with the keyboard, the measurement on 100,000 files, screenshots   |

The layout turns a sorted list of paths into positions. The same set of paths always gives the
same bytes, and a change to a file's contents moves nothing. Only adding or removing files moves
anything.

## Build and test

Use Node 24 or newer.

```sh
npm --prefix ui ci          # install exactly what package-lock.json lists
npm --prefix ui run check   # type check, ESLint, Prettier, unit tests
npm --prefix ui run build   # writes ui/dist
npm --prefix ui run e2e     # browser checks against ui/dist (build first)
npm --prefix ui run dev     # Vite dev server
```

From the repository root, `cargo xtask ui` runs the install and the build, `cargo xtask verify`
runs `check`, the build and the bundle's size limit (1.5 MB gzipped), and
`cargo xtask verify --full` adds the browser checks.

The browser checks use Playwright's Chromium and serve the built page and fixture data from
inside the test, so they need no server and no network. Every response carries the daemon's
security headers (its content security policy, `Referrer-Policy: same-origin` and
`X-Content-Type-Options: nosniff`), and a check fails on any console error, so a script, worker
or request the policy refuses is caught. `e2e/scale.spec.ts` prints the first load time in five
fresh browsers and the frame rate over a scripted pan and zoom on a generated repository of
100,000 files, with the platform it ran on; when `CORTEX_MAP_SCALE_FILE` names a file it writes
them there, and `cargo xtask verify --full` checks them against the map's budgets (a first load
within 2 s every time, 45 frames per second or more at the 5th percentile) on macOS and Linux and
prints them on Windows. `e2e/programs.spec.ts` checks that the map's GPU programs are prepared
while the snapshot loads (see below). `e2e/fit.spec.ts` walks every element at 1440x900 and
390x844 in both themes and fails on content that spills out of its box (outside a declared scroll
area), on text below 12 px and on text not set in Geist; it also delays the font files and checks
that the canvas waits for them. `e2e/steps.spec.ts` answers the questions the map is for with the
keyboard alone, counts the keys and screens each takes, asserts both and prints them as a table.
`e2e/colors.spec.ts` compares every colour token as Chromium paints it with the canvas's channels.
`e2e/receipts.spec.ts` opens the receipts on a cold store, at 29 tasks a side and at 30 a side
with a comparison, checks each headline word for word and that no share, saving or interval shows
without a comparison, reads the chart's sentences with the keyboard, and checks that the receipts
are asked for only when their section opens and that a failure says what failed.
`e2e/live.spec.ts` answers the live view's socket from inside the test and pushes frames: the
markers move and the camera does not, the live sessions list and fly, a refused frame is dropped,
and a lost connection pauses and comes back. `e2e/panels.spec.ts` checks the activity, the
warnings, the health (a report it cannot read too), a file's symbols, the history worker off, a
generated file's commits, files named by an id the map on screen holds another file at, and the
quiet refresh (the file panel staying on its file when a file added before it moves its place), and
confirms and retires facts: the test's stand-in daemon refuses a POST without
the map's own `Origin` header as Chromium sent it, and the check reads that header; that the real
daemon takes the POST is checked against a running daemon.
Setting `CORTEX_SHOTS_DIR` makes `e2e/shots.spec.ts` write screenshots of every state at 1440x900
and 390x844 in both themes into that folder.

## The first load

Before the map can draw, the browser's GPU driver compiles and links its six shader programs
(circles, squares, the live markers' diamonds, dashed lines, glyphs and label backings). deck.gl links them one at a time and
waits for each, and on Direct3D drivers each takes a few hundred milliseconds. While the snapshot
loads, the page therefore builds every layer the map uses, empty and invisible, on a hidden canvas
whose layers start all six links at once (`src/map/prelink.ts`), so the driver compiles them side
by side while the page waits for the daemon. The browser keeps linked programs, and the map's own
canvas then links the same programs from that cache. On a Windows laptop (Intel Iris Xe, Direct3D
11, Chromium 158), at 100,000 files and in fresh browsers, this took the first load from 1.83 to
1.98 s down to 0.89 to 0.95 s over five runs each. The hidden canvas waits for the fonts too, as
its label layers build a glyph atlas the map's canvas shares.

TypeScript is pinned to 6.0.3, the newest release whose JavaScript API the ESLint TypeScript
plugin supports; the same compiler does the type check, so the linter and the check agree.

The build uses relative asset paths so the page works under the daemon's `/<token>/` prefix.
