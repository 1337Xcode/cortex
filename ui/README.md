# Cortex map UI

The map is a web page that shows a repository as a graph: directories are hubs, files sit on
rings around them, and the links Cortex has learned (files changed together, files used together,
files named in work orders) are drawn between files. The Cortex daemon serves the built page on
127.0.0.1, and the page reads its data from the daemon's `api/` endpoints with the access token
that is part of the page's own URL. It asks nothing of any other origin.

This folder holds the page's source. It is written in strict TypeScript with React, Vite,
deck.gl, d3-hierarchy and Zustand.

## What the page shows

- A strip of plain sentences at the top, on one line with the most useful first and the rest
  behind a "Show all" button: the pair of files that changed together most often, the folder
  agents worked on most in the last week (by the tokens attributed to it), how many files agents
  have read, and how many files and folders are indexed. A repository Cortex has learned nothing
  about says so, and the map shows its structure alone.
- The graph. Folders are small squares, files are hollow circles sized by their line count and
  never so large that neighbours touch. A folder's sub-folders sit as close to it as they can
  without overlapping. Learned links are drawn in one accent colour, their kind told by the line
  pattern (solid: changed together; dashed: used together; dotted: named in a work order) and their
  weight by their opacity. Only links at or above the strength set in the legend are drawn, and
  where many links are near the screen only the strongest few of each kind stay strong, the rest
  faint. File outlines are neutral except where they mean something, and each state has a shape
  as well as a colour: a file with a suspect fact is amber with a second ring around it, a file an
  agent read or edited in the last 24 hours is green with a dot in its centre, and a file no agent
  has read has a faint, thinner outline. The legend shows each one.
- Level of detail. Zoomed out, a folder whose rings would be too small to read draws whole, as a
  translucent disc around its square, named with its file count ("orders · 25"); links of its
  files are drawn to it. While rings are only a few pixels apart, each folder draws its own files
  as one disc. A repository of 100,000 files opens on an overview of its top folders.
- Labels where they can be read: folder names wherever they fit, file names once zoomed in, and
  always the hovered or selected file and its linked files. Labels never overlap each other, a
  node, or the page's controls.
- Hovering a file highlights it and its linked files and dims everything else; clicking selects
  it and opens the side panel. Clicking a folder zooms in until it opens. Screen readers hear the
  selection as it changes, not every file the pointer passes.
- Names that would read the same say where they are: two files called `mod.rs` among the links
  show as "mod.rs in orders" and "mod.rs in billing".
- The side panel for the selected file: its path, lines, agent reads, commits and the tokens
  attributed to it (the tokens of agent turns whose last read or edit was this file); its
  strongest links of each kind (with commit counts for files changed together); the facts
  anchored to it, suspect ones marked; the commits its stored history names; and each time Cortex
  named it to an agent, with whether the agent then read or edited it. A "Show the folder" button
  frames the whole folder the file is in, open, which is how the keyboard reaches a folder drawn
  collapsed.
- Text from the repository is shown as text. A path holding an invisible or bidirectional
  formatting character is refused; in commit subjects, refs and facts such characters show as
  `�`. If drawing the page fails, it shows the error instead of a blank page.

## Keyboard

| Key                        | Does                                                               |
| -------------------------- | ------------------------------------------------------------------ |
| `/`                        | Focus the search box                                               |
| Enter (in search)          | Fly to the first result and select it                              |
| Up and Down                | Move through the search results                                    |
| `Z`                        | Fit the whole graph                                                |
| `C`                        | Centre on the selected file, or on the graph                       |
| `+` and `-`                | Zoom in and out by a factor of 1.3                                 |
| Arrow keys                 | Pan, unless a search result has the focus                          |
| Escape                     | Close the search results, or close the side panel and return focus |
| Enter on "Show the folder" | Open the selected file's folder, filling the view                  |

Every control is a native button or input and shows a focus ring. Motion lasts 150 ms and is off
when the system asks for reduced motion. Light and dark themes follow the system setting.

## What is here

| Path            | Holds                                                                                      |
| --------------- | ------------------------------------------------------------------------------------------ |
| `src/api/`      | One client per endpoint, each with a guard that refuses what the contract forbids          |
| `src/state/`    | The Zustand store: load states, the selected and hovered file, the link strength threshold |
| `src/layout/`   | The deterministic radial layout and the level-of-detail helper, both pure functions        |
| `src/map/`      | The deck.gl canvas: encoders from the layout's typed arrays, label placement, the camera   |
| `src/controls/` | Fit, Center and zoom, the legend, the search box and the shortcut table                    |
| `src/panels/`   | The summary strip and the side panel                                                       |
| `src/theme/`    | Design tokens for both themes (every colour, size, opacity and timing) and their CSS       |
| `src/text/`     | Byte-order comparison of strings, number and date formatting, distinct short file names    |
| `e2e/`          | Playwright checks driven with the keyboard, the measurement on 100,000 files, screenshots  |

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

The browser checks use Playwright's Chromium and serve the built page and fixture data from
inside the test, so they need no server and no network. Every response carries the daemon's
security headers (its content security policy, `Referrer-Policy: no-referrer` and
`X-Content-Type-Options: nosniff`), and a check fails on any console error, so a script, worker
or request the policy refuses is caught. `e2e/scale.spec.ts` prints the first load
time and the frame rate over a scripted pan and zoom on a generated repository of 100,000 files,
with the platform it ran on. Setting `CORTEX_SHOTS_DIR` makes `e2e/shots.spec.ts` write
screenshots of every state at 1440x900 and 390x844 in both themes into that folder.

TypeScript is pinned to 6.0.3, the newest release whose JavaScript API the ESLint TypeScript
plugin supports; the same compiler does the type check, so the linter and the check agree.

The build uses relative asset paths so the page works under the daemon's `/<token>/` prefix.
