// Builds the deck.gl layers of one frame from the encoded buffers, bottom to top: the dot grid,
// hub-to-parent lines, spokes, learned links, collapsed hubs, files, hub squares, labels. Every
// geometry layer takes binary attributes straight from the typed arrays.
import { OrthographicView, type Layer, type LayerExtension } from '@deck.gl/core';
import { ScatterplotLayer, TextLayer } from '@deck.gl/layers';
import { CANVAS_SIZE, FONT, LABEL, OPACITY, type ThemeColors } from '../theme/tokens';
import { rgbaOf } from '../theme/oklch';
import { DashedLineLayer } from './dashedLineLayer';
import type { FileBuffers } from './fileEncoding';
import type { HubBuffers } from './hubEncoding';
import type { CanvasLabel } from './labelCandidates';
import type { LineBuffers } from './linkEncoding';
import { SquareLayer } from './squareLayer';

/** The map's one view: a flat camera driven by the pointer; the page handles the keyboard itself. */
export const MAP_VIEW = new OrthographicView({
  id: 'map',
  controller: { keyboard: false, inertia: false },
});

/** Layer ids, which picking uses to tell what was hit. */
export const LAYER_ID = {
  grid: 'grid',
  hubLinks: 'hub-links',
  spokes: 'spokes',
  links: 'links',
  collapsed: 'collapsed-hubs',
  fileDiscs: 'file-discs',
  fileMarks: 'file-marks',
  files: 'files',
  hubs: 'hubs',
  hubLabels: 'hub-labels',
  fileLabels: 'file-labels',
} as const;

/** Everything one frame draws. */
export interface FrameBuffers {
  readonly grid: Float32Array;
  readonly hubLinks: LineBuffers;
  readonly spokes: LineBuffers;
  readonly links: LineBuffers;
  readonly collapsed: HubBuffers;
  readonly fileDiscs: HubBuffers;
  readonly files: FileBuffers;
  /** The suspect rings and recent dots over the files (radius in world units). */
  readonly fileMarks: HubBuffers;
  readonly hubs: HubBuffers;
  readonly labels: readonly CanvasLabel[];
}

/**
 * Props set on every layer of a frame. The map sets none; the hidden canvas that prepares the
 * map's GPU programs makes every layer invisible and adds the extension that does it.
 */
export interface LayerOverrides {
  readonly visible?: boolean;
  readonly extensions?: LayerExtension[];
}

/** Lines with per-row dashes; solid rows have a dash of 0, so every line kind shares one shader. */
function lineLayer(id: string, lines: LineBuffers, overrides: LayerOverrides): Layer {
  return new DashedLineLayer({
    ...overrides,
    id,
    data: {
      length: lines.ids.length,
      attributes: {
        getSourcePosition: { value: lines.sources, size: 2 },
        getTargetPosition: { value: lines.targets, size: 2 },
        getColor: { value: lines.colors, size: 4 },
        getWidth: { value: lines.widths, size: 1 },
        getDashArray: { value: lines.dashes, size: 2 },
      },
    },
    widthUnits: 'pixels',
    // Learned links explain themselves under the pointer; the structure lines do not.
    pickable: id === LAYER_ID.links,
  });
}

function hubLayer(
  id: string,
  hubs: HubBuffers,
  isSquare: boolean,
  overrides: LayerOverrides,
): Layer {
  const isWorldSized = id !== LAYER_ID.hubs;
  const props = {
    ...overrides,
    id,
    data: {
      length: hubs.ids.length,
      attributes: {
        getPosition: { value: hubs.positions, size: 2 },
        getRadius: { value: hubs.radii, size: 1 },
        getFillColor: { value: hubs.fillColors, size: 4 },
        getLineColor: { value: hubs.lineColors, size: 4 },
      },
    },
    radiusUnits: isWorldSized ? ('common' as const) : ('pixels' as const),
    radiusMinPixels: isWorldSized ? CANVAS_SIZE.collapsedMinPx : 0,
    stroked: true,
    filled: true,
    lineWidthUnits: 'pixels' as const,
    getLineWidth: CANVAS_SIZE.collapsedStrokePx,
    pickable: true,
  };
  return isSquare ? new SquareLayer(props) : new ScatterplotLayer(props);
}

function fileLayer(files: FileBuffers, overrides: LayerOverrides): Layer {
  return new ScatterplotLayer({
    ...overrides,
    id: LAYER_ID.files,
    data: {
      length: files.ids.length,
      attributes: {
        getPosition: { value: files.positions, size: 2 },
        getRadius: { value: files.radii, size: 1 },
        getFillColor: { value: files.fillColors, size: 4 },
        getLineColor: { value: files.lineColors, size: 4 },
        getLineWidth: { value: files.lineWidths, size: 1 },
      },
    },
    radiusUnits: 'common',
    radiusMinPixels: CANVAS_SIZE.fileRadiusMinPx,
    stroked: true,
    filled: true,
    lineWidthUnits: 'pixels',
    pickable: true,
  });
}

/** The second channel of the file marks: rings and dots, not pickable, so files below take the pointer. */
function marksLayer(marks: HubBuffers, overrides: LayerOverrides): Layer {
  return new ScatterplotLayer({
    ...overrides,
    id: LAYER_ID.fileMarks,
    data: {
      length: marks.ids.length,
      attributes: {
        getPosition: { value: marks.positions, size: 2 },
        getRadius: { value: marks.radii, size: 1 },
        getFillColor: { value: marks.fillColors, size: 4 },
        getLineColor: { value: marks.lineColors, size: 4 },
      },
    },
    radiusUnits: 'common',
    stroked: true,
    filled: true,
    lineWidthUnits: 'pixels',
    getLineWidth: CANVAS_SIZE.fileStrokePx,
  });
}

function gridLayer(dots: Float32Array, colors: ThemeColors, overrides: LayerOverrides): Layer {
  return new ScatterplotLayer({
    ...overrides,
    id: LAYER_ID.grid,
    data: { length: dots.length / 2, attributes: { getPosition: { value: dots, size: 2 } } },
    radiusUnits: 'pixels',
    getRadius: CANVAS_SIZE.gridDotRadiusPx,
    getFillColor: [...rgbaOf(colors.canvas.gridDot, OPACITY.grid)],
  });
}

/** One kind's labels, or nothing when it has none (an empty text layer makes an empty glyph atlas). */
function labelLayer(
  labels: readonly CanvasLabel[],
  kind: CanvasLabel['kind'],
  colors: ThemeColors,
  overrides: LayerOverrides,
): Layer[] {
  const ofKind = labels.filter((label) => label.kind === kind);
  if (ofKind.length === 0) return [];
  const text = rgbaOf(colors.canvas.label);
  const faint = rgbaOf(colors.canvas.label, OPACITY.labelDimmed);
  const backing = rgbaOf(colors.background, OPACITY.labelBacking);
  const layer = new TextLayer<CanvasLabel>({
    ...overrides,
    id: kind === 'hub' ? LAYER_ID.hubLabels : LAYER_ID.fileLabels,
    data: ofKind,
    getText: (label) => label.text,
    getPosition: (label) => [label.worldX, label.worldY],
    getPixelOffset: (label) => [label.offset[0], label.offset[1]],
    getSize: (label) => label.sizePx,
    getColor: (label) => [...(label.isDimmed ? faint : text)],
    getTextAnchor: (label) => label.anchor,
    getAlignmentBaseline: (label) => label.baseline,
    fontFamily: FONT.sans,
    fontWeight: kind === 'hub' ? LABEL.hubWeight : LABEL.fileWeight,
    characterSet: 'auto',
    background: true,
    getBackgroundColor: [...backing],
    backgroundPadding: LABEL.backingPaddingPx,
    updateTriggers: { getColor: [colors.canvas.label], getBackgroundColor: [colors.background] },
  });
  return [layer];
}

/** The frame's layers, bottom to top, each with `overrides` set. */
export function buildLayers(
  frame: FrameBuffers,
  colors: ThemeColors,
  overrides: LayerOverrides = {},
): Layer[] {
  return [
    gridLayer(frame.grid, colors, overrides),
    hubLayer(LAYER_ID.fileDiscs, frame.fileDiscs, false, overrides),
    lineLayer(LAYER_ID.hubLinks, frame.hubLinks, overrides),
    lineLayer(LAYER_ID.spokes, frame.spokes, overrides),
    lineLayer(LAYER_ID.links, frame.links, overrides),
    hubLayer(LAYER_ID.collapsed, frame.collapsed, false, overrides),
    fileLayer(frame.files, overrides),
    marksLayer(frame.fileMarks, overrides),
    hubLayer(LAYER_ID.hubs, frame.hubs, true, overrides),
    ...labelLayer(frame.labels, 'hub', colors, overrides),
    ...labelLayer(frame.labels, 'file', colors, overrides),
  ];
}
