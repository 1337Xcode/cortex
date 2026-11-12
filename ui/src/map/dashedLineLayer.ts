// A line layer that can dash each line in screen pixels. deck.gl's own dash support lives in a
// package the map does not ship, so this extends the line layer's shaders by injection: the
// vertex shader passes the distance along the line in CSS pixels and the line's dash and gap, and
// the fragment shader discards the gaps. A dash of 0 draws the line solid.
import type { Accessor, DefaultProps } from '@deck.gl/core';
import { LineLayer, type LineLayerProps } from '@deck.gl/layers';

/** The extra prop: dash then gap in pixels for each line. */
interface DashProps {
  readonly getDashArray?: Accessor<unknown, readonly [number, number]>;
}

/** Props of the dashed line layer. */
export type DashedLineLayerProps = LineLayerProps & DashProps;

const VS_DECLARATIONS = `
in vec2 instanceDashArrays;
out vec2 vDashArray;
out float vDashDistance;
`;

// `source`, `target` and `positions` are the line layer's own locals; the line runs from x = 0 to
// x = 1 of `positions`. Clip space to device pixels, then device pixels to CSS pixels.
const VS_MAIN_END = `
vec2 dashSpanPx = (target.xy / target.w - source.xy / source.w) * project.viewportSize * 0.5;
vDashDistance = positions.x * length(dashSpanPx) / project.devicePixelRatio;
vDashArray = instanceDashArrays;
`;

const FS_DECLARATIONS = `
in vec2 vDashArray;
in float vDashDistance;
`;

const FS_MAIN_START = `
if (vDashArray.x > 0.0 && mod(vDashDistance, vDashArray.x + vDashArray.y) > vDashArray.x) {
  discard;
}
`;

function isShaderSet(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null;
}

/** Straight lines with an optional per-line dash pattern in pixels. */
export class DashedLineLayer extends LineLayer<unknown, DashProps> {
  static override layerName = 'DashedLineLayer';

  static override defaultProps: DefaultProps<DashedLineLayerProps> = {
    ...LineLayer.defaultProps,
    getDashArray: { type: 'accessor', value: [0, 0] },
  };

  override getShaders(): Record<string, unknown> {
    const shaders: unknown = super.getShaders();
    if (!isShaderSet(shaders)) throw new Error('the line layer gave no shaders to extend');
    return {
      ...shaders,
      inject: {
        'vs:#decl': VS_DECLARATIONS,
        'vs:#main-end': VS_MAIN_END,
        'fs:#decl': FS_DECLARATIONS,
        'fs:#main-start': FS_MAIN_START,
      },
    };
  }

  override initializeState(): void {
    super.initializeState();
    this.getAttributeManager()?.addInstanced({
      instanceDashArrays: { size: 2, accessor: 'getDashArray', defaultValue: [0, 0] },
    });
  }
}
