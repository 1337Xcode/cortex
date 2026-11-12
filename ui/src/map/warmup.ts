// The layers that prepare the map's GPU programs while its snapshot is still on the way: every
// layer the map draws, built by the map's own `buildLayers` from a frame with no rows and one
// label of each kind, made invisible and given the prelink extension (see prelink.ts). Building
// them makes deck.gl compile each shader and start every link at once (see ProgramWarmup.tsx).
import type { Layer } from '@deck.gl/core';
import { LABEL, type ThemeColors } from '../theme/tokens';
import type { HubBuffers } from './hubEncoding';
import type { CanvasLabel } from './labelCandidates';
import { buildLayers, type FrameBuffers, type LayerOverrides } from './layers';
import type { LineBuffers } from './linkEncoding';
import { PrelinkExtension } from './prelink';

/** Set on every warm-up layer: invisible, since a prelinked layer's pipeline cannot draw. */
const PRELINKED: LayerOverrides = { visible: false, extensions: [new PrelinkExtension()] };

/** Line buffers with no rows. */
function noLines(): LineBuffers {
  return {
    ids: new Uint32Array(0),
    sources: new Float32Array(0),
    targets: new Float32Array(0),
    colors: new Uint8ClampedArray(0),
    widths: new Float32Array(0),
    dashes: new Float32Array(0),
  };
}

/** Hub buffers with no rows. */
function noHubs(): HubBuffers {
  return {
    ids: new Uint32Array(0),
    positions: new Float32Array(0),
    radii: new Float32Array(0),
    fillColors: new Uint8ClampedArray(0),
    lineColors: new Uint8ClampedArray(0),
  };
}

/**
 * A label of `kind` at the origin. A text layer needs at least one character: with none it builds
 * an empty glyph atlas, and `buildLayers` leaves an empty label kind out.
 */
function placeholderLabel(kind: CanvasLabel['kind']): CanvasLabel {
  return {
    key: `warmup-${kind}`,
    text: '0',
    kind,
    family: kind === 'marker' ? 'other' : null,
    worldX: 0,
    worldY: 0,
    sizePx: kind === 'hub' ? LABEL.hubSizePx : LABEL.fileSizePx,
    isDimmed: false,
    side: 'below',
    anchor: 'middle',
    baseline: 'top',
    offset: [0, 0],
  };
}

/** The frame the warm-up layers are built from: no geometry, one label of each kind. */
function warmupFrame(): FrameBuffers {
  return {
    grid: new Float32Array(0),
    hubLinks: noLines(),
    spokes: noLines(),
    links: noLines(),
    collapsed: noHubs(),
    fileDiscs: noHubs(),
    files: {
      ids: new Uint32Array(0),
      positions: new Float32Array(0),
      radii: new Float32Array(0),
      fillColors: new Uint8ClampedArray(0),
      lineColors: new Uint8ClampedArray(0),
      lineWidths: new Float32Array(0),
    },
    fileMarks: noHubs(),
    badges: noHubs(),
    hubs: noHubs(),
    live: {
      sessions: [],
      files: [],
      markers: noHubs(),
      spots: [],
      trails: noLines(),
      stops: noHubs(),
    },
    labels: [placeholderLabel('hub'), placeholderLabel('file'), placeholderLabel('marker')],
  };
}

/** The warm-up layers in `colors`: the map's own layers, empty, invisible and prelinked. */
export function warmupLayers(colors: ThemeColors): Layer[] {
  return buildLayers(warmupFrame(), colors, PRELINKED);
}
