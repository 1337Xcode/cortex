// The deterministic radial layout: a pure function from file paths to positions.
//
// Each directory is a hub. Its own files sit on rings around it in path order. Its
// sub-directories sit around it in name order, each in an angular sector proportional to the
// room its sub-tree needs (its covering radius plus the gap), and each as close to the hub as its
// sector and the hub's own rings allow, so sub-trees never overlap and never sit further out than
// they must. Nothing depends on file contents, the clock or randomness: the same set of paths
// always gives the same bytes.
import type { HierarchyNode } from 'd3-hierarchy';
import { compareBytes, isStrictlySortedBytes } from '../text/byteOrder';
import {
  HUB_PADDING,
  MIN_RINGS,
  RING_SPACING,
  SECTOR_START_ANGLE,
  SLOTS_PER_RING_STEP,
  SUBTREE_GAP,
} from './constants';
import { buildDirectoryTree, type DirectoryNode } from './directoryTree';

const TAU = 2 * Math.PI;

/** Positions for every hub (directory) and file. Hub 0 is the root; hubs are in pre-order. */
export interface MapLayout {
  /** Hub centres in world units. */
  readonly hubX: Float32Array;
  readonly hubY: Float32Array;
  /** The parent hub of each hub; -1 for the root. */
  readonly hubParent: Int32Array;
  /** Radius of a circle around the hub covering its whole sub-tree. */
  readonly hubRadius: Float32Array;
  /** Number of files in the hub's whole sub-tree. */
  readonly hubFileCount: Uint32Array;
  /** The directory path of each hub; empty for the root. */
  readonly hubPath: readonly string[];
  /** File positions in world units, indexed like the sorted path list. */
  readonly fileX: Float32Array;
  readonly fileY: Float32Array;
  /** The hub each file belongs to. */
  readonly fileHub: Uint32Array;
}

type Hub = HierarchyNode<DirectoryNode>;

/** How many rings `fileCount` files need, never fewer than `MIN_RINGS`. */
function ringsFor(fileCount: number): number {
  let rings = 0;
  let capacity = 0;
  while (capacity < fileCount) {
    rings += 1;
    capacity += SLOTS_PER_RING_STEP * rings;
  }
  return Math.max(rings, MIN_RINGS);
}

/** The largest value; a loop, because spreading a huge list into `Math.max` overflows the stack. */
function largest(values: readonly number[]): number {
  let best = 0;
  for (const value of values) best = Math.max(best, value);
  return best;
}

/**
 * The angle of each sub-directory's sector, in order; they add up to a full circle. Each is
 * proportional to the room its sub-tree needs across the sector, its radius plus the gap, so a
 * small sub-tree beside a large one is not pushed far out to fit a sliver.
 */
function sectorSizes(childRadii: readonly number[]): number[] {
  const needs = childRadii.map((radius) => radius + SUBTREE_GAP);
  const total = needs.reduce((sum, need) => sum + need, 0);
  return needs.map((need) => (TAU * need) / total);
}

/**
 * The distance from a hub to one sub-directory's centre: the least that keeps the sub-tree's
 * circle (plus the gap) clear of the hub's own rings and inside its sector. A sector wider than
 * a half turn bounds nothing, so its half angle is capped at a quarter turn.
 */
function childDistance(ownRadius: number, sector: number, radius: number): number {
  const halfAngle = Math.min(sector / 2, Math.PI / 2);
  return Math.max(ownRadius + radius + SUBTREE_GAP, (radius + SUBTREE_GAP) / Math.sin(halfAngle));
}

/** Sizes of every hub, computed from the leaves up. */
interface HubSizes {
  readonly fileCount: Uint32Array;
  readonly ownRadius: Float64Array;
  /** Each hub's distance from its parent hub; 0 for the root. */
  readonly parentDistance: Float64Array;
  readonly radius: Float64Array;
}

function measureHubs(hubs: readonly Hub[], indexOf: ReadonlyMap<Hub, number>): HubSizes {
  const fileCount = new Uint32Array(hubs.length);
  const ownRadius = new Float64Array(hubs.length);
  const distance = new Float64Array(hubs.length);
  const radius = new Float64Array(hubs.length);
  // Pre-order lists a parent before its children, so the reverse visits children first.
  for (let h = hubs.length - 1; h >= 0; h -= 1) {
    const hub = hubs[h];
    if (hub === undefined) continue;
    const children = (hub.children ?? []).map((child) => indexOf.get(child) ?? 0);
    const direct = hub.data.files.length;
    ownRadius[h] = ringsFor(direct) * RING_SPACING;
    fileCount[h] = direct + children.reduce((sum, c) => sum + (fileCount[c] ?? 0), 0);
    if (children.length === 0) {
      radius[h] = (ownRadius[h] ?? 0) + HUB_PADDING;
      continue;
    }
    const sectors = sectorSizes(children.map((c) => radius[c] ?? 0));
    const reaches = children.map((c, i) => {
      distance[c] = childDistance(ownRadius[h] ?? 0, sectors[i] ?? TAU, radius[c] ?? 0);
      return (distance[c] ?? 0) + (radius[c] ?? 0);
    });
    radius[h] = Math.max(ownRadius[h] ?? 0, largest(reaches)) + HUB_PADDING;
  }
  return { fileCount, ownRadius, parentDistance: distance, radius };
}

/** Writes the positions of a hub's own files onto rings around its centre. */
function placeFiles(hub: Hub, x: number, y: number, layout: MapLayout, hubIndex: number): void {
  let ring = 1;
  let slot = 0;
  for (const file of hub.data.files) {
    const slots = SLOTS_PER_RING_STEP * ring;
    const angle = (TAU * slot) / slots;
    layout.fileX[file] = x + ring * RING_SPACING * Math.cos(angle);
    layout.fileY[file] = y + ring * RING_SPACING * Math.sin(angle);
    layout.fileHub[file] = hubIndex;
    slot += 1;
    if (slot === slots) {
      ring += 1;
      slot = 0;
    }
  }
}

function emptyLayout(hubs: readonly Hub[], fileTotal: number): MapLayout {
  return {
    hubX: new Float32Array(hubs.length),
    hubY: new Float32Array(hubs.length),
    hubParent: new Int32Array(hubs.length).fill(-1),
    hubRadius: new Float32Array(hubs.length),
    hubFileCount: new Uint32Array(hubs.length),
    hubPath: hubs.map((hub) => hub.data.path),
    fileX: new Float32Array(fileTotal),
    fileY: new Float32Array(fileTotal),
    fileHub: new Uint32Array(fileTotal),
  };
}

function placeHubs(
  hubs: readonly Hub[],
  indexOf: ReadonlyMap<Hub, number>,
  sizes: HubSizes,
  fileTotal: number,
): MapLayout {
  const layout = emptyLayout(hubs, fileTotal);
  // Positions stay in float64 until stored, so a child's offset from its parent is not
  // compounded by float32 rounding at every level.
  const x = new Float64Array(hubs.length);
  const y = new Float64Array(hubs.length);
  for (const [h, hub] of hubs.entries()) {
    const children = hub.children ?? [];
    layout.hubX[h] = x[h] ?? 0;
    layout.hubY[h] = y[h] ?? 0;
    layout.hubRadius[h] = sizes.radius[h] ?? 0;
    layout.hubFileCount[h] = sizes.fileCount[h] ?? 0;
    placeFiles(hub, x[h] ?? 0, y[h] ?? 0, layout, h);
    const indexes = children.map((child) => indexOf.get(child) ?? 0);
    const sectors = sectorSizes(indexes.map((c) => sizes.radius[c] ?? 0));
    let start = SECTOR_START_ANGLE;
    for (const [i, child] of indexes.entries()) {
      const sector = sectors[i] ?? 0;
      const angle = start + sector / 2;
      const distance = sizes.parentDistance[child] ?? 0;
      x[child] = (x[h] ?? 0) + distance * Math.cos(angle);
      y[child] = (y[h] ?? 0) + distance * Math.sin(angle);
      layout.hubParent[child] = h;
      start += sector;
    }
  }
  return layout;
}

/**
 * Lays out the files at `paths`. Outputs are indexed by position in byte order, which is the
 * file id for a snapshot that satisfies the contract; unsorted input is sorted first, so the
 * same set of paths gives the same bytes in any order.
 */
export function layoutPaths(paths: readonly string[]): MapLayout {
  // INVARIANT: deterministic (the layout depends on the set of paths, never on their order)
  const sorted = isStrictlySortedBytes(paths) ? paths : [...paths].sort(compareBytes);
  const root = buildDirectoryTree(sorted);
  const hubs: Hub[] = [];
  root.eachBefore((hub) => hubs.push(hub));
  const indexOf = new Map<Hub, number>(hubs.map((hub, index) => [hub, index]));
  return placeHubs(hubs, indexOf, measureHubs(hubs, indexOf), sorted.length);
}
