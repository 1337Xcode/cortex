// A scatterplot layer that draws squares instead of circles, for folder hubs. The scatterplot's
// fragment shader measures distance from the centre with `length`; measuring it as the larger of
// the two axis distances instead turns every circle into the square around it.
import { ScatterplotLayer } from '@deck.gl/layers';

/** The circle's distance measure in the scatterplot fragment shader. */
const CIRCLE_DISTANCE = 'length(unitPosition)';

/** The square's distance measure: the larger axis distance. */
const SQUARE_DISTANCE = 'max(abs(unitPosition.x), abs(unitPosition.y))';

function hasFragmentShader(value: unknown): value is { readonly fs: string } {
  return (
    typeof value === 'object' && value !== null && 'fs' in value && typeof value.fs === 'string'
  );
}

/** Squares centred on each position, sized like the scatterplot's circles (radius = half side). */
export class SquareLayer extends ScatterplotLayer {
  static override layerName = 'SquareLayer';

  override getShaders(): Record<string, unknown> {
    const shaders: unknown = super.getShaders();
    if (!hasFragmentShader(shaders) || !shaders.fs.includes(CIRCLE_DISTANCE)) {
      // Fail loudly if a deck.gl upgrade changes the shader, instead of drawing circles silently.
      throw new Error('the scatterplot shader no longer measures distance as expected');
    }
    return { ...shaders, fs: shaders.fs.replace(CIRCLE_DISTANCE, SQUARE_DISTANCE) };
  }
}
