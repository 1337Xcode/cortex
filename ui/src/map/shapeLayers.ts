// Scatterplot layers that draw other shapes than circles: squares for folder hubs and diamonds for
// live agent markers. The scatterplot's fragment shader measures distance from the centre with
// `length`; measuring it as the larger of the two axis distances turns every circle into the
// square around it, and as their sum into the diamond inside that square, whose half-diagonal is
// the circle's radius. Stroke and fill follow the same measure, so outlines keep their width.
import { ScatterplotLayer } from '@deck.gl/layers';

/** The circle's distance measure in the scatterplot fragment shader. */
const CIRCLE_DISTANCE = 'length(unitPosition)';

/** The square's distance measure: the larger axis distance. */
const SQUARE_DISTANCE = 'max(abs(unitPosition.x), abs(unitPosition.y))';

/** The diamond's distance measure: the sum of the axis distances. */
const DIAMOND_DISTANCE = '(abs(unitPosition.x) + abs(unitPosition.y))';

function hasFragmentShader(value: unknown): value is { readonly fs: string } {
  return (
    typeof value === 'object' && value !== null && 'fs' in value && typeof value.fs === 'string'
  );
}

/** The scatterplot's shaders with its distance measure replaced by `measure`. */
function withDistance(shaders: unknown, measure: string): Record<string, unknown> {
  if (!hasFragmentShader(shaders) || !shaders.fs.includes(CIRCLE_DISTANCE)) {
    // Fail loudly if a deck.gl upgrade changes the shader, instead of drawing circles silently.
    throw new Error('the scatterplot shader no longer measures distance as expected');
  }
  return { ...shaders, fs: shaders.fs.replace(CIRCLE_DISTANCE, measure) };
}

/** Squares centred on each position, sized like the scatterplot's circles (radius = half side). */
export class SquareLayer extends ScatterplotLayer {
  static override layerName = 'SquareLayer';

  override getShaders(): Record<string, unknown> {
    return withDistance(super.getShaders(), SQUARE_DISTANCE);
  }
}

/** Diamonds centred on each position, sized like the circles (radius = half the diagonal). */
export class DiamondLayer extends ScatterplotLayer {
  static override layerName = 'DiamondLayer';

  override getShaders(): Record<string, unknown> {
    return withDistance(super.getShaders(), DIAMOND_DISTANCE);
  }
}
