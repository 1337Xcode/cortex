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
- The overview panel on the left: the agent sessions running in the repository now, every summary
  sentence, the strongest pairs of files changed together (by the link's strength, then its
  commits, the order the summary's strongest pair is chosen by; each row shows its commits and
  says "changed with mod.rs in 12 of 21 commits" on hover), the busiest files (by the tokens
  attributed to them, or by commits before agents worked on any) and the files that need attention
  (a suspect fact). Each list shows five rows and then "Show N more", and each row is a button that
  flies to its file and selects it. Each section collapses to its title and count. The live
  sessions open while one runs and say "No agent session is running in this repository",
  collapsed, while none does. On a narrow screen the overview is a bottom sheet opened from the top
  bar.
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
  each file through four shades of the accent on a log scale from the fewest to the most any file
  has. Coupling steps how many learned links each file has, of any kind and strength, the same
  way. Recent lights the files an agent read or edited in the 24 hours before the map loaded. In
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
  commits"). Hovering a file also highlights it and its linked files and dims everything else;
  clicking selects it and opens its panel. Clicking a folder zooms in until it opens. Screen readers
  hear the selection as it changes, not every file the pointer passes.
- Names that would read the same say where they are: two files called `mod.rs` among the links
  show as "mod.rs in orders" and "mod.rs in billing".
- The panel of the selected file: the breadcrumb of its folders, each a button that frames that
  folder whole and open (the keyboard's way into a folder drawn collapsed); its name and path; its
  lines, agent reads, commits and the tokens attributed to it (the tokens of agent turns whose last
  read or edit was this file), in tabular figures. Then sections that collapse to their title and
  count. Its strongest links of each kind and the facts anchored to it are open, as they answer the
  questions asked most (what usually changes with this file, and which rules apply to it). The
  commits its stored history names, in pages of ten ("11 to 20 of 34"), and each time Cortex named
  it to an agent, with whether the agent then read or edited it, open when asked. Its commits and
  tokens say what they count on hover and keyboard focus: the commits are only those of the history
  Cortex reads (its most recent commits, within an age, leaving out commits that changed too many
  files, each limit as the daemon reports it), so 0 never reads as "never committed". "Open in
  editor" opens the file in the editor the daemon names; the page shows the link only when its
  scheme is not a web, script, data or file address.
- Long text (paths, names, commit subjects) is cut at a path or word boundary with an ellipsis and
  shown whole on hover and on keyboard focus; nothing scrolls sideways.
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
security headers (its content security policy, `Referrer-Policy: no-referrer` and
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
and a lost connection pauses and comes back.
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
