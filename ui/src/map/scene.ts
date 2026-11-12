// Everything the map derives once per snapshot: the layout, each file's drawn radius and name,
// each hub's name, the graph's bounds, and the learned links indexed by file. Frames read from
// here and never walk the snapshot's objects again.
import type { GraphSnapshot } from '../api/graph';
import { RING_SPACING, SLOTS_PER_RING_STEP } from '../layout/constants';
import { layoutPaths, type MapLayout } from '../layout/radial';
import { CANVAS_SIZE } from '../theme/tokens';

/** An axis-aligned box in world units. */
export interface Bounds {
  readonly minX: number;
  readonly minY: number;
  readonly maxX: number;
  readonly maxY: number;
}

/** Learned links grouped by file: the links of file f are `linkIds[offsets[f]]` up to `offsets[f + 1]`. */
export interface Adjacency {
  readonly offsets: Uint32Array;
  readonly linkIds: Uint32Array;
}

/** The per-snapshot data every frame is drawn from. */
export interface Scene {
  readonly snapshot: GraphSnapshot;
  readonly layout: MapLayout;
  /** Each file's circle radius in world units. */
  readonly fileRadius: Float32Array;
  /** Each file's name: the last path segment. */
  readonly fileName: readonly string[];
  /** Radius in world units of the rings holding each hub's own files; 0 for a hub with none. */
  readonly hubOwnRadius: Float32Array;
  /** Each hub's name: the last segment of its directory; empty for the root. */
  readonly hubName: readonly string[];
  readonly bounds: Bounds;
  readonly adjacency: Adjacency;
  /** Whether any file has been read, so never-read files are worth dimming (the Fog). */
  readonly hasReads: boolean;
}

/** A file's circle radius in world units: grows with the square root of its lines, clamped. */
export function fileRadiusFor(lines: number): number {
  const radius = CANVAS_SIZE.fileRadiusMin + CANVAS_SIZE.fileRadiusPerSqrtLine * Math.sqrt(lines);
  return Math.min(CANVAS_SIZE.fileRadiusMax, radius);
}

/**
 * The radius of the rings that hold `fileCount` files: ring k holds `SLOTS_PER_RING_STEP * k`
 * slots, so k rings hold `SLOTS_PER_RING_STEP * k * (k + 1) / 2`.
 */
export function ownRingsRadius(fileCount: number): number {
  let rings = 0;
  while ((SLOTS_PER_RING_STEP * rings * (rings + 1)) / 2 < fileCount) rings += 1;
  return rings * RING_SPACING;
}

/** Each hub's own-files ring radius, from the files' hubs. */
function ownRadii(layout: MapLayout): Float32Array {
  const counts = new Uint32Array(layout.hubParent.length);
  for (const hub of layout.fileHub) counts[hub] = (counts[hub] ?? 0) + 1;
  return Float32Array.from(counts, (count) => ownRingsRadius(count));
}

function lastSegment(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

/** The box around every hub centre and every file circle. */
function boundsOf(layout: MapLayout, fileRadius: Float32Array): Bounds {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  const grow = (x: number, y: number, r: number): void => {
    minX = Math.min(minX, x - r);
    minY = Math.min(minY, y - r);
    maxX = Math.max(maxX, x + r);
    maxY = Math.max(maxY, y + r);
  };
  for (let h = 0; h < layout.hubX.length; h += 1) grow(layout.hubX[h] ?? 0, layout.hubY[h] ?? 0, 0);
  for (let f = 0; f < layout.fileX.length; f += 1) {
    grow(layout.fileX[f] ?? 0, layout.fileY[f] ?? 0, fileRadius[f] ?? 0);
  }
  return { minX, minY, maxX, maxY };
}

/** Indexes the links by both ends, in link order, so a file's links are one contiguous run. */
export function buildAdjacency(snapshot: GraphSnapshot): Adjacency {
  const fileCount = snapshot.files.length;
  const offsets = new Uint32Array(fileCount + 1);
  for (const link of snapshot.links) {
    offsets[link.from + 1] = (offsets[link.from + 1] ?? 0) + 1;
    offsets[link.to + 1] = (offsets[link.to + 1] ?? 0) + 1;
  }
  for (let f = 0; f < fileCount; f += 1) offsets[f + 1] = (offsets[f + 1] ?? 0) + (offsets[f] ?? 0);
  const cursor = offsets.slice(0, fileCount);
  const linkIds = new Uint32Array(snapshot.links.length * 2);
  for (const [id, link] of snapshot.links.entries()) {
    for (const end of [link.from, link.to]) {
      const slot = cursor[end] ?? 0;
      linkIds[slot] = id;
      cursor[end] = slot + 1;
    }
  }
  return { offsets, linkIds };
}

/** Builds the scene for a snapshot that passed the guard (files sorted, ids equal to indexes). */
export function buildScene(snapshot: GraphSnapshot): Scene {
  const layout = layoutPaths(snapshot.files.map((file) => file.path));
  const fileRadius = new Float32Array(snapshot.files.length);
  for (const [id, file] of snapshot.files.entries()) fileRadius[id] = fileRadiusFor(file.lines);
  return {
    snapshot,
    layout,
    fileRadius,
    fileName: snapshot.files.map((file) => lastSegment(file.path)),
    hubOwnRadius: ownRadii(layout),
    hubName: layout.hubPath.map(lastSegment),
    bounds: boundsOf(layout, fileRadius),
    adjacency: buildAdjacency(snapshot),
    hasReads: snapshot.files.some((file) => file.reads > 0),
  };
}
