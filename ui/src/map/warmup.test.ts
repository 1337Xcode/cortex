// Tests for the warm-up layers: they are the map's own layers, every kind of them, so every
// program the map links is prepared; and none of them can be drawn.
import { describe, expect, it } from 'vitest';
import { DARK_COLORS, LIGHT_COLORS } from '../theme/tokens';
import { LAYER_ID } from './layers';
import { PrelinkExtension } from './prelink';
import { warmupLayers } from './warmup';

describe('warmupLayers', () => {
  it('builds one layer of every id the map draws, in both themes', () => {
    const everyId = Object.values(LAYER_ID).sort();
    for (const colors of [DARK_COLORS, LIGHT_COLORS]) {
      expect(
        warmupLayers(colors)
          .map((layer) => layer.id)
          .sort(),
      ).toEqual(everyId);
    }
  });

  it('makes every layer invisible and gives it the prelink extension', () => {
    for (const layer of warmupLayers(DARK_COLORS)) {
      expect(layer.props.visible).toBe(false);
      expect(
        layer.props.extensions.some((extension) => extension instanceof PrelinkExtension),
      ).toBe(true);
    }
  });
});
