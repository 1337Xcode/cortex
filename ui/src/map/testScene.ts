// A small scene for the map's tests: the seven-file tree the layout tests work out by hand, with
// links the test supplies. Positions and radii are in the layout tests' comment: hubs 0 root
// (covering radius 120), 1 a (15), 2 b (50), 3 b/deep (15), 4 c (15); README.md sits at (10, 0).
import type { GraphFile, GraphLink, GraphSnapshot } from '../api/graph';
import { defaultView } from '../views/default';
import { paintView, type ViewPaint } from '../views/paint';
import { buildScene, type Scene } from './scene';

/** The seven paths in byte order; their index is their id. */
export const SEVEN_PATHS = [
  'README.md',
  'a/one.rs',
  'a/two.rs',
  'b/deep/leaf.rs',
  'b/mid.rs',
  'b/zed.rs',
  'c/only.rs',
] as const;

/** A file with every count at zero, changed by `fields`. */
export function testFile(id: number, path: string, fields: Partial<GraphFile> = {}): GraphFile {
  return {
    id,
    path,
    lines: 100,
    interest: 0,
    tokens: 0,
    touched_ms: null,
    facts: 0,
    suspect: 0,
    reads: 0,
    commits: 0,
    ...fields,
  };
}

/** The seven-file snapshot with `links` and per-file changes by id. */
export function sevenFileSnapshot(
  links: readonly GraphLink[] = [],
  changes: ReadonlyMap<number, Partial<GraphFile>> = new Map(),
): GraphSnapshot {
  const files = SEVEN_PATHS.map((path, id) => testFile(id, path, changes.get(id)));
  return { version: 1, files, links };
}

/** The seven-file scene with `links`. */
export function sevenFileScene(links: readonly GraphLink[] = []): Scene {
  return buildScene(sevenFileSnapshot(links));
}

/** The default view of `scene`, "now" at `nowMs`, for a frame style. */
export function defaultPaint(scene: Scene, nowMs = 0): ViewPaint {
  return paintView(defaultView, scene, nowMs);
}

/** A link with no commits, for kinds other than co-change. */
export function testLink(
  from: number,
  to: number,
  kind: GraphLink['kind'],
  weight: number,
): GraphLink {
  return { from, to, kind, weight, commits: kind === 'co_change' ? 1 : 0 };
}
