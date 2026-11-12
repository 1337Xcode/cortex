// Builds the deck.gl layers of one frame from the encoded buffers, bottom to top: the dot grid,
// hub-to-parent lines, spokes, learned links, collapsed hubs, files, hub squares, labels. Every
// geometry layer takes binary attributes straight from the typed arrays.
import type { Layer } from '@deck.gl/core';
import { ScatterplotLayer, TextLayer } from '@deck.gl/layers';
import { CANVAS_SIZE, FONT_FAMILY, LABEL, OPACITY, type ThemeColors } from '../theme/tokens';
import { rgba } from '../theme/rgba';
import { DashedLineLayer } from './dashedLineLayer';
import type { FileBuffers } from './fileEncoding';
import type { HubBuffers } from './hubEncoding';
import type { CanvasLabel } from './labelCandidates';
import type { LineBuffers } from './linkEncoding';
import { SquareLayer } from './squareLayer';

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

/** Lines with per-row dashes; solid rows have a dash of 0, so every line kind shares one shader. */
function lineLayer(id: string, lines: LineBuffers): Layer {
  return new DashedLineLayer({
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
  });
}

function hubLayer(id: string, hubs: HubBuffers, isSquare: boolean): Layer {
  const isWorldSized = id !== LAYER_ID.hubs;
  const props = {
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

function fileLayer(files: FileBuffers): Layer {
  return new ScatterplotLayer({
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
function marksLayer(marks: HubBuffers): Layer {
  return new ScatterplotLayer({
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

function gridLayer(dots: Float32Array, colors: ThemeColors): Layer {
  return new ScatterplotLayer({
    id: LAYER_ID.grid,
    data: { length: dots.length / 2, attributes: { getPosition: { value: dots, size: 2 } } },
    radiusUnits: 'pixels',
    getRadius: CANVAS_SIZE.gridDotRadiusPx,
    getFillColor: [...rgba(colors.canvas.gridDot, OPACITY.grid)],
  });
}

/** One kind's labels, or nothing when it has none (an empty text layer makes an empty glyph atlas). */
function labelLayer(
  labels: readonly CanvasLabel[],
  kind: CanvasLabel['kind'],
  colors: ThemeColors,
): Layer[] {
  const ofKind = labels.filter((label) => label.kind === kind);
  if (ofKind.length === 0) return [];
  const text = rgba(colors.canvas.label);
  const faint = rgba(colors.canvas.label, OPACITY.labelDimmed);
  const backing = rgba(colors.background, OPACITY.labelBacking);
  const layer = new TextLayer<CanvasLabel>({
    id: kind === 'hub' ? LAYER_ID.hubLabels : LAYER_ID.fileLabels,
    data: ofKind,
    getText: (label) => label.text,
    getPosition: (label) => [label.worldX, label.worldY],
    getPixelOffset: (label) => [label.offset[0], label.offset[1]],
    getSize: (label) => label.sizePx,
    getColor: (label) => [...(label.isDimmed ? faint : text)],
    getTextAnchor: (label) => label.anchor,
    getAlignmentBaseline: (label) => label.baseline,
    fontFamily: FONT_FAMILY,
    fontWeight: kind === 'hub' ? LABEL.hubWeight : LABEL.fileWeight,
    characterSet: 'auto',
    background: true,
    getBackgroundColor: [...backing],
    backgroundPadding: LABEL.backingPaddingPx,
    updateTriggers: { getColor: [colors.canvas.label], getBackgroundColor: [colors.background] },
  });
  return [layer];
}

/** The frame's layers, bottom to top. */
export function buildLayers(frame: FrameBuffers, colors: ThemeColors): Layer[] {
  return [
    gridLayer(frame.grid, colors),
    hubLayer(LAYER_ID.fileDiscs, frame.fileDiscs, false),
    lineLayer(LAYER_ID.hubLinks, frame.hubLinks),
    lineLayer(LAYER_ID.spokes, frame.spokes),
    lineLayer(LAYER_ID.links, frame.links),
    hubLayer(LAYER_ID.collapsed, frame.collapsed, false),
    fileLayer(frame.files),
    marksLayer(frame.fileMarks),
    hubLayer(LAYER_ID.hubs, frame.hubs, true),
    ...labelLayer(frame.labels, 'hub', colors),
    ...labelLayer(frame.labels, 'file', colors),
  ];
}
