# Cortex map UI

The map is a web page that shows a repository as a graph: directories are hubs, files sit on
rings around them, and the links Cortex has learned (files changed together, files used together,
files named in work orders) are drawn between files. The Cortex daemon serves the built page on
127.0.0.1 and the page reads its data from the daemon's `api/graph` endpoint, with the access
token that is part of the page's own URL.

This folder holds the page's source. It is written in strict TypeScript with React, Vite,
deck.gl, d3-hierarchy and Zustand.

## What is here

| Path          | Holds                                                                                 |
| ------------- | ------------------------------------------------------------------------------------- |
| `src/api/`    | The graph snapshot types, the guard that checks the daemon's JSON, and the fetch call |
| `src/state/`  | The Zustand store: load state (loading, error, empty, ready) and the selected file    |
| `src/layout/` | The deterministic radial layout and the level-of-detail helper, both pure functions   |
| `src/theme/`  | Design tokens for the light and dark themes, and the code that applies them           |
| `src/text/`   | Byte-order comparison of strings, matching the order the daemon sorts paths in        |

The layout turns a sorted list of paths into positions. The same set of paths always gives the
same bytes, and a change to a file's contents moves nothing. Only adding or removing files moves
anything.

## Build and test

Use Node 24 or newer.

```sh
npm --prefix ui ci          # install exactly what package-lock.json lists
npm --prefix ui run check   # type check, ESLint, Prettier, unit tests
npm --prefix ui run build   # writes ui/dist
npm --prefix ui run dev     # Vite dev server
```

TypeScript is pinned to 6.0.3, the newest release whose JavaScript API the ESLint TypeScript
plugin supports; the same compiler does the type check, so the linter and the check agree.

The build uses relative asset paths so the page works under the daemon's `/<token>/` prefix.
