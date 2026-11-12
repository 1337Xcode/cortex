// Which learned links draw at full strength. While few links are near the screen, all of them do.
// Past a density, only the strongest few of each kind near the screen keep their strength (and the
// hovered or selected file's links, which the encoder always draws strong); the rest draw faint,
// so a dense folder reads as its strongest couplings instead of a haze. On the overview, where
// files are too small to draw one by one, only the strongest few links of the frame are drawn at
// all, until the person zooms in or points at or selects a file.
import { LINK_KINDS, type LinkKind } from '../api/graph';
import type { Camera } from './camera';
import type { ScreenSize } from './labels';
import { drawnEnd, isLinkDrawn } from './linkEncoding';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** An axis-aligned box in world units. */
export interface WorldBox {
  readonly minX: number;
  readonly minY: number;
  readonly maxX: number;
  readonly maxY: number;
}

/** The links that keep their strength, by link id (1 marks one), and what becomes of the rest. */
export interface Prominence {
  readonly marked: Uint8Array;
  /** The rest draw faint, or are not drawn at all. */
  readonly rest: 'faint' | 'hidden';
}

/** How many links near the screen make the frame dense, and how many per kind stay strong then. */
export interface ProminenceLimits {
  readonly denseAbove: number;
  readonly strongestPerKind: number;
}

/**
 * The world box links are counted in: twice the screen around the camera, with the centre
 * snapped to a quarter of the screen and the zoom to half steps, so the box (and the work that
 * depends on it) changes only a few times while the camera moves.
 */
export function nearBox(camera: Camera, screen: ScreenSize): WorldBox {
  const scale = 2 ** (Math.floor(camera.zoom * 2) / 2);
  const tileX = Math.max(screen.width, 1) / 4 / scale;
  const tileY = Math.max(screen.height, 1) / 4 / scale;
  const x = Math.round(camera.x / tileX) * tileX;
  const y = Math.round(camera.y / tileY) * tileY;
  const halfWidth = Math.max(screen.width, 1) / scale;
  const halfHeight = Math.max(screen.height, 1) / scale;
  return { minX: x - halfWidth, minY: y - halfHeight, maxX: x + halfWidth, maxY: y + halfHeight };
}

/** A box's corners as text, for memo keys. */
export function boxKey(box: WorldBox): string {
  return `${box.minX},${box.minY},${box.maxX},${box.maxY}`;
}

function isInBox([x, y]: [number, number], box: WorldBox): boolean {
  return x >= box.minX && x <= box.maxX && y >= box.minY && y <= box.maxY;
}

/** The drawn links with an end in `box`, grouped by kind. */
function nearLinks(
  scene: Scene,
  visibility: Visibility,
  minStrength: number,
  box: WorldBox,
): Record<LinkKind, number[]> {
  const near: Record<LinkKind, number[]> = { co_change: [], used_together: [], work_order: [] };
  for (const [id, link] of scene.snapshot.links.entries()) {
    if (!isLinkDrawn(link, visibility, minStrength)) continue;
    const ends = [drawnEnd(scene, visibility, link.from), drawnEnd(scene, visibility, link.to)];
    if (ends.some((end) => isInBox(end, box))) near[link.kind].push(id);
  }
  return near;
}

/** Link ids strongest first; equal weights by link id. */
function strongestFirst(scene: Scene, ids: readonly number[]): number[] {
  const weightOf = (id: number): number => scene.snapshot.links[id]?.weight ?? 0;
  // INVARIANT: deterministic (ties by link id, never by the order the browser sorts in)
  return [...ids].sort((a, b) => weightOf(b) - weightOf(a) || a - b);
}

/**
 * The links that draw at full strength, the rest faint, or null when every drawn link does (the
 * frame is not dense): the strongest `strongestPerKind` of each kind near the screen.
 */
export function prominentLinks(
  scene: Scene,
  visibility: Visibility,
  options: {
    readonly minStrength: number;
    readonly box: WorldBox;
    readonly limits: ProminenceLimits;
  },
): Prominence | null {
  const near = nearLinks(scene, visibility, options.minStrength, options.box);
  const total = LINK_KINDS.reduce((sum, kind) => sum + near[kind].length, 0);
  if (total <= options.limits.denseAbove) return null;
  const marked = new Uint8Array(scene.snapshot.links.length);
  for (const kind of LINK_KINDS) {
    const strongest = strongestFirst(scene, near[kind]);
    for (const id of strongest.slice(0, options.limits.strongestPerKind)) marked[id] = 1;
  }
  return { marked, rest: 'faint' };
}

/**
 * The overview's links: the `count` strongest drawn links near the screen, of any kind, and none
 * of the rest, so the first sight of a repository is its strongest couplings, not a tangle.
 */
export function overviewLinks(
  scene: Scene,
  visibility: Visibility,
  options: { readonly minStrength: number; readonly box: WorldBox; readonly count: number },
): Prominence {
  const near = nearLinks(scene, visibility, options.minStrength, options.box);
  const marked = new Uint8Array(scene.snapshot.links.length);
  const all = LINK_KINDS.flatMap((kind) => near[kind]);
  for (const id of strongestFirst(scene, all).slice(0, options.count)) marked[id] = 1;
  return { marked, rest: 'hidden' };
}
