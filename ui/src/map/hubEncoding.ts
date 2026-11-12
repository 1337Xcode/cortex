// Encodes the folder structure: open hubs as small squares, collapsed hubs as one node sized by
// their file count, a faint dashed line from each drawn hub to its parent, and a faint spoke from
// each drawn file to its hub. All neutral, except that in a view that tints folders a folder drawn
// whole (or as one disc of its files) is edged in its strongest file's tone; outside the focus they
// dim with everything else.
import { RING_SPACING } from '../layout/constants';
import { CANVAS_SIZE, OPACITY } from '../theme/tokens';
import { rgbaOf, type Rgba } from '../theme/oklch';
import { NO_TONE } from '../views/folderTones';
import { toneColor } from '../views/view';
import { focusShare, writeRgba, type FrameStyle } from './frameStyle';
import type { LineBuffers } from './linkEncoding';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** Hub nodes as binary attributes; row i is hub `ids[i]`. */
export interface HubBuffers {
  readonly ids: Uint32Array;
  readonly positions: Float32Array;
  /** Radius (half side for squares) in pixels. */
  readonly radii: Float32Array;
  readonly fillColors: Uint8ClampedArray;
  readonly lineColors: Uint8ClampedArray;
}

/** For each hub, 1 when it holds or stands in for a file of the focus neighbourhood. */
export function focusHubs(scene: Scene, visibility: Visibility, style: FrameStyle): Uint8Array {
  const marked = new Uint8Array(scene.layout.hubParent.length);
  const neighbourhood = style.neighbourhood;
  if (neighbourhood === null) return marked;
  for (const id of [neighbourhood.focus, ...neighbourhood.neighbours]) {
    const hub = scene.layout.fileHub[id] ?? 0;
    marked[hub] = 1;
    const standIn = visibility.fileStandIn[id] ?? -1;
    if (standIn >= 0) marked[standIn] = 1;
  }
  return marked;
}

/** The colours of one kind of hub node. */
interface NodeColors {
  readonly fill: Rgba;
  readonly line: Rgba;
  readonly marked: Uint8Array;
  /** Each hub's edge in its view tone, or null to keep `line` for every hub. */
  readonly toneLine: ((hub: number) => Rgba) | null;
}

/**
 * Each hub's edge colour in the active view, from its strongest tone in `tones` (own files or the
 * whole sub-tree); null when the view keeps folders neutral. A folder with no file keeps `line`.
 */
function toneLineOf(
  style: FrameStyle,
  pick: 'own' | 'tree',
  line: Rgba,
): ((hub: number) => Rgba) | null {
  const folders = style.view.folders;
  if (folders === null) return null;
  const tones = folders[pick];
  const palette = style.view.encoding.tones.map((tone) =>
    rgbaOf(toneColor(tone.role, style.colors)),
  );
  return (hub) => {
    const tone = tones[hub] ?? NO_TONE;
    return tone === NO_TONE ? line : (palette[tone] ?? line);
  };
}

function encodeNodes(
  scene: Scene,
  ids: Uint32Array,
  radiusOf: (hub: number) => number,
  colors: NodeColors,
  style: FrameStyle,
): HubBuffers {
  const count = ids.length;
  const buffers: HubBuffers = {
    ids,
    positions: new Float32Array(count * 2),
    radii: new Float32Array(count),
    fillColors: new Uint8ClampedArray(count * 4),
    lineColors: new Uint8ClampedArray(count * 4),
  };
  const { fill, line, toneLine } = colors;
  for (let row = 0; row < count; row += 1) {
    const hub = ids[row] ?? 0;
    buffers.positions[row * 2] = scene.layout.hubX[hub] ?? 0;
    buffers.positions[row * 2 + 1] = scene.layout.hubY[hub] ?? 0;
    buffers.radii[row] = radiusOf(hub);
    const share = focusShare(colors.marked[hub] === 1, style);
    writeRgba(buffers.fillColors, row, fill, share);
    writeRgba(buffers.lineColors, row, toneLine === null ? line : toneLine(hub), share);
  }
  return buffers;
}

/** Every drawn hub's square, open or collapsed (a collapsed one also has its disc), in hub order. */
export function encodeHubSquares(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  marked: Uint8Array,
): HubBuffers {
  const ids = Uint32Array.from([...visibility.openHubs, ...visibility.collapsedHubs]).sort();
  const hub = rgbaOf(style.colors.canvas.hub);
  const colors = { fill: hub, line: hub, marked, toneLine: null };
  return encodeNodes(scene, ids, () => CANVAS_SIZE.hubHalfSidePx, colors, style);
}

/**
 * A collapsed hub's radius in world units: a share of the circle its sub-tree covers. The covering
 * circle's area grows with the files inside it, so bigger sub-trees draw bigger, and sibling
 * circles never overlap, so neither do collapsed nodes.
 */
export function collapsedRadius(scene: Scene, hub: number): number {
  return (scene.layout.hubRadius[hub] ?? 0) * CANVAS_SIZE.collapsedShareOfReach;
}

/**
 * The collapsed hubs as round nodes sized by the sub-tree they stand for (radius in world units):
 * a translucent disc in the hub colour with a solid hub-coloured edge, the hub's square at its
 * centre, so a whole folder never reads as a file.
 */
export function encodeCollapsedHubs(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  marked: Uint8Array,
): HubBuffers {
  const hub = style.colors.canvas.hub;
  const line = rgbaOf(hub);
  const toneLine = toneLineOf(style, 'tree', line);
  const colors = { fill: rgbaOf(hub, OPACITY.collapsedFill), line, marked, toneLine };
  const radiusOf = (hub: number): number => collapsedRadius(scene, hub);
  return encodeNodes(scene, visibility.collapsedHubs, radiusOf, colors, style);
}

/** While files are not shown one by one, each open hub's own files as one disc (radius in world units). */
export function encodeFileDiscs(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  marked: Uint8Array,
): HubBuffers {
  const holders = visibility.areFilesShown
    ? new Uint32Array(0)
    : visibility.openHubs.filter((hub) => (scene.hubOwnRadius[hub] ?? 0) > 0);
  const canvas = style.colors.canvas;
  const line = rgbaOf(canvas.unread);
  const toneLine = toneLineOf(style, 'own', line);
  const colors = { fill: rgbaOf(canvas.collapsedFill), line, marked, toneLine };
  return encodeNodes(scene, holders, (hub) => scene.hubOwnRadius[hub] ?? 0, colors, style);
}

function emptyLines(count: number): LineBuffers {
  return {
    ids: new Uint32Array(count),
    sources: new Float32Array(count * 2),
    targets: new Float32Array(count * 2),
    colors: new Uint8ClampedArray(count * 4),
    widths: new Float32Array(count),
    dashes: new Float32Array(count * 2),
  };
}

/** A dashed line from every drawn hub (open or collapsed) to its parent; row ids are child hubs. */
export function encodeHubLinks(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  marked: Uint8Array,
  dash: readonly [number, number],
): LineBuffers {
  const { hubParent, hubX, hubY } = scene.layout;
  const drawn: number[] = [];
  for (let h = 1; h < hubParent.length; h += 1) {
    const standIn = visibility.hubStandIn[h] ?? -1;
    if (standIn < 0 || standIn === h) drawn.push(h);
  }
  const buffers = emptyLines(drawn.length);
  const color = rgbaOf(style.colors.canvas.hubLink, OPACITY.hubLink);
  for (const [row, hub] of drawn.entries()) {
    const parent = hubParent[hub] ?? 0;
    buffers.ids[row] = hub;
    buffers.sources.set([hubX[parent] ?? 0, hubY[parent] ?? 0], row * 2);
    buffers.targets.set([hubX[hub] ?? 0, hubY[hub] ?? 0], row * 2);
    writeRgba(
      buffers.colors,
      row,
      color,
      focusShare(marked[hub] === 1 && marked[parent] === 1, style),
    );
    buffers.widths[row] = CANVAS_SIZE.hubLinkWidthPx;
    buffers.dashes.set(dash, row * 2);
  }
  return buffers;
}

/** A solid spoke from every drawn file of a small folder to its hub; row ids are files. */
export function encodeSpokes(scene: Scene, visibility: Visibility, style: FrameStyle): LineBuffers {
  const { fileHub, fileX, fileY, hubX, hubY } = scene.layout;
  // A folder with more rings than this reads by its rings alone; its spokes would be a haze.
  const maxOwnRadius = CANVAS_SIZE.spokeMaxRings * RING_SPACING;
  const files = visibility.visibleFiles.filter(
    (file) => (scene.hubOwnRadius[fileHub[file] ?? 0] ?? 0) <= maxOwnRadius,
  );
  const buffers = emptyLines(files.length);
  const color = rgbaOf(style.colors.canvas.hubLink, OPACITY.spoke);
  for (let row = 0; row < files.length; row += 1) {
    const file = files[row] ?? 0;
    const hub = fileHub[file] ?? 0;
    buffers.ids[row] = file;
    buffers.sources.set([hubX[hub] ?? 0, hubY[hub] ?? 0], row * 2);
    buffers.targets.set([fileX[file] ?? 0, fileY[file] ?? 0], row * 2);
    writeRgba(
      buffers.colors,
      row,
      color,
      focusShare(style.neighbourhood?.isMember[file] === 1, style),
    );
    buffers.widths[row] = CANVAS_SIZE.spokeWidthPx;
  }
  return buffers;
}
