// Gathers the labels a frame could draw, in the order they matter: the focus file, then its
// neighbours, then hub names (bigger sub-trees first), then file names once the rings are far
// enough apart to read them (longer files first). Each offers a spot below, above, right and left
// of its node; the collision rule in `labels.ts` keeps the ones that fit, where they fit.
import { CANVAS_SIZE, LABEL } from '../theme/tokens';
import { toScreen, type Camera } from './camera';
import type { Neighbourhood } from './highlight';
import { collapsedRadius } from './hubEncoding';
import { placeLabels, type LabelCandidate, type ScreenBox, type ScreenSize } from './labels';
import type { Scene } from './scene';
import { folderLabel } from '../text/vocabulary';
import type { Visibility } from './visibility';

/** Measures the width in pixels of `text` at a type size and weight. */
export type MeasureText = (text: string, sizePx: number, weight: number) => number;

/** Where a label sits against its node. */
export type LabelSide = 'below' | 'above' | 'right' | 'left';

/** The sides a label tries, in order of preference. */
const LABEL_SIDES: readonly LabelSide[] = ['below', 'above', 'right', 'left'];

/** A node that wants a label, before placement. */
export interface LabelNode extends LabelCandidate {
  /** Identifies the node: `h<hub>` or `f<file>`, stable across frames. */
  readonly key: string;
  readonly text: string;
  readonly kind: 'hub' | 'file';
  readonly worldX: number;
  readonly worldY: number;
  /** Pixels from the node's centre to the near edge of its label. */
  readonly distancePx: number;
  readonly sizePx: number;
  /** Drawn faint: a focus is set and the label's node is outside it. */
  readonly isDimmed: boolean;
}

/** A label chosen for the canvas, with what the text layer needs to draw it. */
export interface CanvasLabel {
  readonly key: string;
  readonly text: string;
  readonly kind: 'hub' | 'file';
  readonly worldX: number;
  readonly worldY: number;
  readonly sizePx: number;
  readonly isDimmed: boolean;
  readonly side: LabelSide;
  readonly anchor: 'start' | 'middle' | 'end';
  readonly baseline: 'top' | 'center' | 'bottom';
  /** The label's offset from its node in pixels, x right and y down. */
  readonly offset: readonly [number, number];
}

/** Everything one frame's labels depend on. */
export interface LabelInputs {
  readonly scene: Scene;
  readonly visibility: Visibility;
  readonly camera: Camera;
  readonly screen: ScreenSize;
  /** The part of the screen the page's chrome leaves clear; labels stay inside it. */
  readonly area: ScreenBox;
  /** Boxes of the page's chrome over the canvas (the controls); labels stay off them. */
  readonly chrome: readonly ScreenBox[];
  readonly neighbourhood: Neighbourhood | null;
  /** 1 for hubs holding a focus file, from `focusHubs`. */
  readonly markedHubs: Uint8Array;
  readonly measure: MeasureText;
}

const PRIORITY = { focus: 4e9, neighbour: 3e9, hub: 2e9 } as const;

/** A label's width, height and distance from its node's centre, in pixels. */
export interface LabelSize {
  readonly width: number;
  readonly height: number;
  readonly distance: number;
}

/** The box of a label on `side` of a node at screen point (x, y). */
export function spotOf(side: LabelSide, x: number, y: number, size: LabelSize): ScreenBox {
  const { width, height, distance } = size;
  switch (side) {
    case 'below':
      return { left: x - width / 2, top: y + distance, width, height };
    case 'above':
      return { left: x - width / 2, top: y - distance - height, width, height };
    case 'right':
      return { left: x + distance, top: y - height / 2, width, height };
    case 'left':
      return { left: x - distance - width, top: y - height / 2, width, height };
  }
}

type NodeFields = Pick<
  LabelNode,
  'key' | 'text' | 'kind' | 'worldX' | 'worldY' | 'priority' | 'isDimmed'
>;

function candidate(inputs: LabelInputs, node: NodeFields, radiusPx: number): LabelNode | null {
  const [x, y] = toScreen(inputs.camera, inputs.screen, node.worldX, node.worldY);
  if (x < 0 || y < 0 || x > inputs.screen.width || y > inputs.screen.height) return null;
  const isHub = node.kind === 'hub';
  const sizePx = isHub ? LABEL.hubSizePx : LABEL.fileSizePx;
  const width = inputs.measure(node.text, sizePx, isHub ? LABEL.hubWeight : LABEL.fileWeight);
  const size = { width, height: sizePx * LABEL.lineHeight, distance: radiusPx + LABEL.gapPx };
  const spots = LABEL_SIDES.map((side) => spotOf(side, x, y, size));
  return { ...node, spots, distancePx: size.distance, sizePx };
}

/** An open hub's drawn radius in pixels: its square, or its file disc while files are not shown. */
function openHubRadiusPx(inputs: LabelInputs, hub: number): number {
  if (inputs.visibility.areFilesShown) return CANVAS_SIZE.hubHalfSidePx;
  const discPx = (inputs.scene.hubOwnRadius[hub] ?? 0) * 2 ** inputs.camera.zoom;
  return Math.max(CANVAS_SIZE.hubHalfSidePx, discPx);
}

/** A collapsed hub's drawn radius in pixels. */
function collapsedRadiusPx(inputs: LabelInputs, hub: number): number {
  const radiusPx = collapsedRadius(inputs.scene, hub) * 2 ** inputs.camera.zoom;
  return Math.max(radiusPx, CANVAS_SIZE.collapsedMinPx);
}

function fileRadiusPx(inputs: LabelInputs, id: number): number {
  const scale = 2 ** inputs.camera.zoom;
  return Math.max((inputs.scene.fileRadius[id] ?? 0) * scale, CANVAS_SIZE.fileRadiusMinPx);
}

function fileCandidate(inputs: LabelInputs, id: number, priority: number): LabelNode | null {
  if ((inputs.visibility.fileStandIn[id] ?? 0) >= 0) return null;
  const { scene, neighbourhood } = inputs;
  const node = {
    key: `f${id}`,
    text: scene.fileName[id] ?? '',
    kind: 'file' as const,
    worldX: scene.layout.fileX[id] ?? 0,
    worldY: scene.layout.fileY[id] ?? 0,
    priority,
    isDimmed: neighbourhood !== null && neighbourhood.isMember[id] !== 1,
  };
  return candidate(inputs, node, fileRadiusPx(inputs, id));
}

/** A folder's label: its name, and for a collapsed folder its file count ("orders · 25 files"). */
export function hubLabelText(scene: Scene, hub: number, isCollapsed: boolean): string {
  const name = scene.hubName[hub] ?? '';
  if (name === '') return name;
  return folderLabel(name, isCollapsed ? (scene.layout.hubFileCount[hub] ?? 0) : null);
}

function hubCandidate(
  inputs: LabelInputs,
  hub: number,
  radiusPx: number,
  isCollapsed: boolean,
): LabelNode | null {
  const { scene, neighbourhood } = inputs;
  const text = hubLabelText(scene, hub, isCollapsed);
  if (text === '') return null;
  const node = {
    key: `h${hub}`,
    text,
    kind: 'hub' as const,
    worldX: scene.layout.hubX[hub] ?? 0,
    worldY: scene.layout.hubY[hub] ?? 0,
    priority: PRIORITY.hub + (scene.layout.hubFileCount[hub] ?? 0),
    isDimmed: neighbourhood !== null && inputs.markedHubs[hub] !== 1,
  };
  return candidate(inputs, node, radiusPx);
}

function pushIf(list: LabelNode[], label: LabelNode | null): void {
  if (label !== null) list.push(label);
}

/** Every label the frame could draw, before the collision rule. */
function gatherCandidates(inputs: LabelInputs): LabelNode[] {
  const list: LabelNode[] = [];
  const { neighbourhood, visibility, scene } = inputs;
  if (neighbourhood !== null) {
    pushIf(list, fileCandidate(inputs, neighbourhood.focus, PRIORITY.focus));
    for (const id of neighbourhood.neighbours) {
      pushIf(list, fileCandidate(inputs, id, PRIORITY.neighbour));
    }
  }
  for (const hub of visibility.openHubs) {
    pushIf(list, hubCandidate(inputs, hub, openHubRadiusPx(inputs, hub), false));
  }
  for (const hub of visibility.collapsedHubs) {
    // A folder drawn smaller than a word is left unnamed: its name would crowd its neighbours'.
    const radiusPx = collapsedRadiusPx(inputs, hub);
    if (radiusPx * 2 < LABEL.collapsedMinLabelPx) continue;
    pushIf(list, hubCandidate(inputs, hub, radiusPx, true));
  }
  if (2 ** inputs.camera.zoom >= LABEL.fileMinPixelsPerUnit) {
    for (const id of visibility.visibleFiles) {
      if (neighbourhood?.isMember[id] === 1) continue;
      pushIf(list, fileCandidate(inputs, id, scene.snapshot.files[id]?.lines ?? 0));
    }
  }
  return list;
}

/** The text layer's anchor, baseline and offset for a label on `side`, `distance` pixels out. */
function drawingOf(
  side: LabelSide,
  distance: number,
): Pick<CanvasLabel, 'anchor' | 'baseline' | 'offset'> {
  switch (side) {
    case 'below':
      return { anchor: 'middle', baseline: 'top', offset: [0, distance] };
    case 'above':
      return { anchor: 'middle', baseline: 'bottom', offset: [0, -distance] };
    case 'right':
      return { anchor: 'start', baseline: 'center', offset: [distance, 0] };
    case 'left':
      return { anchor: 'end', baseline: 'center', offset: [-distance, 0] };
  }
}

/** The square around a node of radius `radiusPx` at a world point, on screen. */
function nodeBox(inputs: LabelInputs, x: number, y: number, radiusPx: number): ScreenBox {
  const [left, top] = toScreen(inputs.camera, inputs.screen, x, y);
  return { left: left - radiusPx, top: top - radiusPx, width: radiusPx * 2, height: radiusPx * 2 };
}

function isOnScreen(box: ScreenBox, screen: ScreenSize): boolean {
  const right = box.left + box.width;
  const bottom = box.top + box.height;
  return right >= 0 && bottom >= 0 && box.left <= screen.width && box.top <= screen.height;
}

/** The drawn nodes on screen, which labels must not cover. */
function nodeObstacles(inputs: LabelInputs): ScreenBox[] {
  const { scene, visibility } = inputs;
  const { fileX, fileY, hubX, hubY } = scene.layout;
  const boxes: ScreenBox[] = [];
  const push = (box: ScreenBox): void => {
    if (isOnScreen(box, inputs.screen)) boxes.push(box);
  };
  for (const id of visibility.visibleFiles) {
    push(nodeBox(inputs, fileX[id] ?? 0, fileY[id] ?? 0, fileRadiusPx(inputs, id)));
  }
  for (const hub of visibility.openHubs) {
    push(nodeBox(inputs, hubX[hub] ?? 0, hubY[hub] ?? 0, openHubRadiusPx(inputs, hub)));
  }
  for (const hub of visibility.collapsedHubs) {
    push(nodeBox(inputs, hubX[hub] ?? 0, hubY[hub] ?? 0, collapsedRadiusPx(inputs, hub)));
  }
  return boxes;
}

/**
 * The labels to draw this frame: the candidates that pass the collision rule, where they fit. The
 * hovered or selected file is named first and may cover other nodes (its backing keeps it
 * readable): in a dense ring every side of it is another file, and the file the person is looking
 * at must never go unnamed. Everything else keeps clear of the nodes and of that name.
 */
export function chooseLabels(inputs: LabelInputs): CanvasLabel[] {
  const candidates = gatherCandidates(inputs);
  const rules = { area: inputs.area, paddingPx: LABEL.paddingPx };
  const focus = placeLabels(
    candidates.filter((node) => node.priority === PRIORITY.focus),
    { ...rules, obstacles: inputs.chrome, maxCount: 1 },
  );
  const focusBoxes = focus.flatMap(({ candidate, spot }) => candidate.spots[spot] ?? []);
  const others = placeLabels(
    candidates.filter((node) => node.priority !== PRIORITY.focus),
    {
      ...rules,
      obstacles: [...inputs.chrome, ...nodeObstacles(inputs), ...focusBoxes],
      maxCount: LABEL.maxCount - focus.length,
    },
  );
  return [...focus, ...others].map(({ candidate: node, spot }) => {
    const side = LABEL_SIDES[spot] ?? 'below';
    const { key, text, kind, worldX, worldY, sizePx, isDimmed } = node;
    const drawing = drawingOf(side, node.distancePx);
    return { key, text, kind, worldX, worldY, sizePx, isDimmed, side, ...drawing };
  });
}
