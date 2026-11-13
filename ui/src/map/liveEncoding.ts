// Encodes the live sessions: for each, a diamond (a shape no file or folder has) around its newest
// file in its agent's colour, and a trail back through the files it worked on before, each step
// fainter than the one after it. A file drawn inside a folder (zoomed out) is marked at that
// folder. Sessions on the same spot draw nested diamonds, each a little larger. A file is marked
// only where the snapshot holds the path the frame gave; one it does not hold (the index changed
// since the map loaded) is left out, never marked at another file's id. Live state never moves the
// camera or the layout: it only adds these marks over what is drawn.
import { MAX_LIVE_FILES, type LiveSession } from '../api/live';
import { CANVAS_SIZE, OPACITY, type ThemeColors } from '../theme/tokens';
import { rgbaOf } from '../theme/oklch';
import { agentFamily } from './agents';
import { writeRgba } from './frameStyle';
import { listedLiveFiles } from './liveFiles';
import type { HubBuffers } from './hubEncoding';
import type { LineBuffers } from './linkEncoding';
import type { Scene } from './scene';
import type { Visibility } from './visibility';

/** Where a session's marker is drawn and how large, for its label. */
export interface MarkerSpot {
  /** Index of the session in `LiveBuffers.sessions`. */
  readonly session: number;
  /** The file the marker is drawn around, or null when a folder stands in for it. */
  readonly file: number | null;
  readonly worldX: number;
  readonly worldY: number;
  /** Half the diamond's diagonal, in pixels. */
  readonly radiusPx: number;
}

/** The live sessions as drawn: markers, trails and the dots on each earlier file. */
export interface LiveBuffers {
  /** The sessions drawn, in id order; each of the arrays below indexes into this. */
  readonly sessions: readonly LiveSession[];
  /** The known files of each drawn session, newest first. */
  readonly files: readonly (readonly number[])[];
  /** Row i marks session `ids[i]`; radius in pixels. */
  readonly markers: HubBuffers;
  readonly spots: readonly MarkerSpot[];
  /** Row i is a step of session `ids[i]`'s trail. */
  readonly trails: LineBuffers;
  /** Row i is an earlier file of session `ids[i]`; radius in pixels. */
  readonly stops: HubBuffers;
}

/** A point of the canvas a file is drawn at, and the circle the marker must clear. */
interface Anchor {
  readonly key: string;
  /** The file drawn there, or null for a folder standing in for it. */
  readonly file: number | null;
  readonly x: number;
  readonly y: number;
  /** The drawn circle's radius in world units; 0 for a folder standing in for the file. */
  readonly radius: number;
}

function anchorOf(scene: Scene, visibility: Visibility, id: number): Anchor {
  const standIn = visibility.fileStandIn[id] ?? -1;
  const { fileX, fileY, hubX, hubY } = scene.layout;
  if (standIn < 0) {
    return {
      key: `f${id}`,
      file: id,
      x: fileX[id] ?? 0,
      y: fileY[id] ?? 0,
      radius: scene.fileRadius[id] ?? 0,
    };
  }
  const [x, y] = [hubX[standIn] ?? 0, hubY[standIn] ?? 0];
  return { key: `h${standIn}`, file: null, x, y, radius: 0 };
}

/**
 * Half the diagonal, in pixels, of a diamond around a circle of `radius` world units at `scale`
 * pixels per unit: a diamond's sides clear a circle once its half-diagonal is the radius times
 * the square root of 2. The `stack`-th session on one spot draws `stack` steps larger.
 */
export function markerRadiusPx(radius: number, scale: number, stack: number): number {
  const around = radius * scale * Math.SQRT2 + CANVAS_SIZE.markerGapPx;
  return Math.max(around, CANVAS_SIZE.markerMinPx) + stack * CANVAS_SIZE.markerStackPx;
}

/** The trail's opacity at step `index` (0 the newest): even steps from newest to oldest. */
export function trailOpacity(index: number): number {
  const share = Math.min(index, MAX_LIVE_FILES - 1) / (MAX_LIVE_FILES - 1);
  return OPACITY.trailNewest + (OPACITY.trailOldest - OPACITY.trailNewest) * share;
}

function nodeBuffers(count: number): HubBuffers {
  return {
    ids: new Uint32Array(count),
    positions: new Float32Array(count * 2),
    radii: new Float32Array(count),
    fillColors: new Uint8ClampedArray(count * 4),
    lineColors: new Uint8ClampedArray(count * 4),
  };
}

function lineBuffers(count: number): LineBuffers {
  return {
    ids: new Uint32Array(count),
    sources: new Float32Array(count * 2),
    targets: new Float32Array(count * 2),
    colors: new Uint8ClampedArray(count * 4),
    widths: new Float32Array(count),
    dashes: new Float32Array(count * 2),
  };
}

/** Each drawn session's anchors, newest first, and the sessions with at least one known file. */
function anchorsOf(
  scene: Scene,
  visibility: Visibility,
  sessions: readonly LiveSession[],
): { drawn: LiveSession[]; files: number[][]; anchors: Anchor[][] } {
  const drawn: LiveSession[] = [];
  const files: number[][] = [];
  const anchors: Anchor[][] = [];
  for (const session of sessions) {
    const known = listedLiveFiles(session, scene.snapshot.files).map((entry) => entry.id);
    if (known.length === 0) continue;
    drawn.push(session);
    files.push(known);
    anchors.push(known.map((id) => anchorOf(scene, visibility, id)));
  }
  return { drawn, files, anchors };
}

/** Encodes `sessions` over the frame `visibility` draws, at `scale` pixels per world unit. */
export function encodeLive(
  scene: Scene,
  visibility: Visibility,
  sessions: readonly LiveSession[],
  view: { readonly colors: ThemeColors; readonly scale: number },
): LiveBuffers {
  const { drawn, files, anchors } = anchorsOf(scene, visibility, sessions);
  const markers = nodeBuffers(drawn.length);
  const spots: MarkerSpot[] = [];
  const stacked = new Map<string, number>();
  const trailRows: [number, Anchor, Anchor, number][] = [];
  const stopRows: [number, Anchor, number][] = [];
  for (const [index, session] of drawn.entries()) {
    const color = rgbaOf(view.colors.agents[agentFamily(session.agent)]);
    const path = anchors[index] ?? [];
    const [newest] = path;
    if (newest === undefined) continue;
    const stack = stacked.get(newest.key) ?? 0;
    stacked.set(newest.key, stack + 1);
    const radiusPx = markerRadiusPx(newest.radius, view.scale, stack);
    markers.ids[index] = index;
    markers.positions.set([newest.x, newest.y], index * 2);
    markers.radii[index] = radiusPx;
    writeRgba(markers.fillColors, index, color, OPACITY.markerFill);
    writeRgba(markers.lineColors, index, color, 1);
    spots.push({ session: index, file: newest.file, worldX: newest.x, worldY: newest.y, radiusPx });
    for (let step = 1; step < path.length; step += 1) {
      const from = path[step - 1];
      const to = path[step];
      if (from === undefined || to === undefined) continue;
      stopRows.push([index, to, trailOpacity(step)]);
      if (from.key !== to.key) trailRows.push([index, from, to, trailOpacity(step - 1)]);
    }
  }
  return {
    sessions: drawn,
    files,
    markers,
    spots,
    trails: encodeTrails(trailRows, drawn, view.colors),
    stops: encodeStops(stopRows, drawn, view.colors),
  };
}

function encodeTrails(
  rows: readonly [number, Anchor, Anchor, number][],
  sessions: readonly LiveSession[],
  colors: ThemeColors,
): LineBuffers {
  const buffers = lineBuffers(rows.length);
  for (const [row, [session, from, to, opacity]] of rows.entries()) {
    const agent = sessions[session]?.agent ?? '';
    buffers.ids[row] = session;
    buffers.sources.set([from.x, from.y], row * 2);
    buffers.targets.set([to.x, to.y], row * 2);
    writeRgba(buffers.colors, row, rgbaOf(colors.agents[agentFamily(agent)]), opacity);
    buffers.widths[row] = CANVAS_SIZE.trailWidthPx;
  }
  return buffers;
}

function encodeStops(
  rows: readonly [number, Anchor, number][],
  sessions: readonly LiveSession[],
  colors: ThemeColors,
): HubBuffers {
  const buffers = nodeBuffers(rows.length);
  for (const [row, [session, at, opacity]] of rows.entries()) {
    const agent = sessions[session]?.agent ?? '';
    const color = rgbaOf(colors.agents[agentFamily(agent)]);
    buffers.ids[row] = session;
    buffers.positions.set([at.x, at.y], row * 2);
    buffers.radii[row] = CANVAS_SIZE.trailStopPx;
    writeRgba(buffers.fillColors, row, color, opacity);
    writeRgba(buffers.lineColors, row, color, 0);
  }
  return buffers;
}
