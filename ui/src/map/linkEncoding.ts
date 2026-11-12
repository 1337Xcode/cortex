// Encodes the learned links at or above the strength threshold into typed arrays. Every link is
// drawn in the one accent; its kind is told by its dash pattern and its weight by its opacity
// (0.2 to 0.6). An end hidden in a collapsed hub is drawn at that hub, and a link whose two ends
// fall in the same collapsed hub is not drawn. Links of the focus file draw on top, at full
// strength; the rest dim while a focus is set.
import type { GraphLink, LinkKind } from '../api/graph';
import { CANVAS_SIZE, LINK_DASH_PX, OPACITY } from '../theme/tokens';
import { rgba } from '../theme/rgba';
import { focusShare, writeRgba, type FrameStyle } from './frameStyle';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** Line segments as binary attributes; row i is link `ids[i]` of the snapshot. */
export interface LineBuffers {
  readonly ids: Uint32Array;
  readonly sources: Float32Array;
  readonly targets: Float32Array;
  readonly colors: Uint8ClampedArray;
  readonly widths: Float32Array;
  /** Dash then gap in pixels per row; 0 and 0 draws solid. */
  readonly dashes: Float32Array;
}

/** A link's opacity from its weight: `linkMin` at 0, `linkMax` at 1. */
export function linkOpacity(weight: number): number {
  return OPACITY.linkMin + (OPACITY.linkMax - OPACITY.linkMin) * weight;
}

/** The dash and gap of a kind in pixels, 0 and 0 for solid. */
export function dashOf(kind: LinkKind): readonly [number, number] {
  const pattern = LINK_DASH_PX[kind];
  return pattern.length === 2 ? pattern : [0, 0];
}

/** Where file `id` is drawn: itself, or the hub that stands in for it. */
export function drawnEnd(scene: Scene, visibility: Visibility, id: number): [number, number] {
  const standIn = visibility.fileStandIn[id] ?? -1;
  const { fileX, fileY, hubX, hubY } = scene.layout;
  return standIn < 0 ? [fileX[id] ?? 0, fileY[id] ?? 0] : [hubX[standIn] ?? 0, hubY[standIn] ?? 0];
}

/** Whether a link is drawn: at or above the threshold, and not inside one stand-in hub. */
export function isLinkDrawn(link: GraphLink, visibility: Visibility, minStrength: number): boolean {
  if (link.weight < minStrength) return false;
  const from = visibility.fileStandIn[link.from] ?? -1;
  return from < 0 || from !== (visibility.fileStandIn[link.to] ?? -1);
}

/** The drawn link ids: faint ones first, then full-strength ones, the focus file's links last. */
function drawOrder(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  prominent: Uint8Array | null,
): Uint32Array {
  const focus = style.neighbourhood?.focus ?? -1;
  const layers: [number[], number[], number[]] = [[], [], []];
  for (const [id, link] of scene.snapshot.links.entries()) {
    if (!isLinkDrawn(link, visibility, style.minStrength)) continue;
    const isFocused = link.from === focus || link.to === focus;
    const isStrong = prominent === null || prominent[id] === 1;
    layers[isFocused ? 2 : isStrong ? 1 : 0].push(id);
  }
  return Uint32Array.from(layers.flat());
}

/**
 * Encodes the learned links `visibility` and the threshold leave on screen. Links `prominent`
 * does not mark (when it is not null) draw at `OPACITY.linkFaint`.
 */
export function encodeLinks(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  prominent: Uint8Array | null,
): LineBuffers {
  const ids = drawOrder(scene, visibility, style, prominent);
  const count = ids.length;
  const buffers: LineBuffers = {
    ids,
    sources: new Float32Array(count * 2),
    targets: new Float32Array(count * 2),
    colors: new Uint8ClampedArray(count * 4),
    widths: new Float32Array(count),
    dashes: new Float32Array(count * 2),
  };
  const accent = rgba(style.colors.accent);
  const focus = style.neighbourhood?.focus ?? -1;
  for (let row = 0; row < count; row += 1) {
    const link = scene.snapshot.links[ids[row] ?? 0];
    if (link === undefined) continue;
    buffers.sources.set(drawnEnd(scene, visibility, link.from), row * 2);
    buffers.targets.set(drawnEnd(scene, visibility, link.to), row * 2);
    const isFocused = link.from === focus || link.to === focus;
    const isStrong = prominent === null || prominent[ids[row] ?? 0] === 1;
    const weightAlpha = isStrong ? linkOpacity(link.weight) : OPACITY.linkFaint;
    const alpha = isFocused ? OPACITY.linkFocused : weightAlpha;
    writeRgba(buffers.colors, row, accent, alpha * focusShare(isFocused, style));
    buffers.widths[row] = isFocused ? CANVAS_SIZE.linkFocusedWidthPx : CANVAS_SIZE.linkWidthPx;
    const [dash, gap] = dashOf(link.kind);
    buffers.dashes[row * 2] = dash;
    buffers.dashes[row * 2 + 1] = gap;
  }
  return buffers;
}

/** How many links of each kind are at or above the threshold, whatever the zoom. */
export function countLinksByKind(
  links: readonly GraphLink[],
  minStrength: number,
): Readonly<Record<LinkKind, number>> {
  const counts: Record<LinkKind, number> = { co_change: 0, used_together: 0, work_order: 0 };
  for (const link of links) if (link.weight >= minStrength) counts[link.kind] += 1;
  return counts;
}
