// Converts token colours (`#rrggbb`) into the 0 to 255 channel values deck.gl draws with.

/** Red, green, blue and alpha, each 0 to 255. */
export type Rgba = readonly [number, number, number, number];

/**
 * The channels of a `#rrggbb` colour with `alpha` from 0 to 1. A token that is not `#rrggbb` is a
 * programming error and throws, so a typo can never draw as black.
 */
export function rgba(hex: string, alpha = 1): Rgba {
  if (!/^#[0-9a-fA-F]{6}$/.test(hex)) throw new Error(`not a #rrggbb colour: ${hex}`);
  const channel = (offset: number): number => parseInt(hex.slice(offset, offset + 2), 16);
  return [channel(1), channel(3), channel(5), Math.round(Math.min(1, Math.max(0, alpha)) * 255)];
}
