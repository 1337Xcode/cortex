// Colours as the tokens write them, in OKLCH (perceptual lightness, chroma and hue), and the two
// forms the page needs: the CSS text the stylesheet reads, and the 0 to 255 sRGB channels deck.gl
// draws the canvas with. The conversion is OKLCH to OKLab to linear sRGB to gamma-encoded sRGB, with
// the matrices of the OKLab definition (Ottosson, 2020) that CSS Color 4 also uses.

/** A colour in OKLCH: lightness 0 to 1, chroma from 0, hue in degrees. */
export interface Oklch {
  readonly l: number;
  readonly c: number;
  readonly h: number;
}

/** Red, green, blue and alpha, each 0 to 255. */
export type Rgba = readonly [number, number, number, number];

/** A token colour; `oklch(0.77, 0.12, 290)` reads like the CSS it becomes. */
export function oklch(l: number, c: number, h: number): Oklch {
  return { l, c, h };
}

/** The colour as CSS: `oklch(0.77 0.12 290)`. */
export function cssOf(color: Oklch): string {
  return `oklch(${color.l} ${color.c} ${color.h})`;
}

/**
 * The colour in linear sRGB, each channel nominally 0 to 1; a channel outside that range means the
 * colour lies outside the sRGB gamut.
 */
export function linearSrgbOf(color: Oklch): readonly [number, number, number] {
  const radians = (color.h * Math.PI) / 180;
  const a = color.c * Math.cos(radians);
  const b = color.c * Math.sin(radians);
  const l = (color.l + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (color.l - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (color.l - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

/** How far outside 0 to 1 a linear channel may sit and still count as inside the gamut. */
const GAMUT_TOLERANCE = 0.0005;

/** Whether the colour can be shown in sRGB without clipping a channel. */
export function isInSrgb(color: Oklch): boolean {
  return linearSrgbOf(color).every(
    (channel) => channel >= -GAMUT_TOLERANCE && channel <= 1 + GAMUT_TOLERANCE,
  );
}

/** The sRGB transfer function: a linear channel to its gamma-encoded value, both 0 to 1. */
function encode(linear: number): number {
  const clipped = Math.min(1, Math.max(0, linear));
  return clipped <= 0.0031308 ? 12.92 * clipped : 1.055 * clipped ** (1 / 2.4) - 0.055;
}

/** The colour's sRGB channels with `alpha` from 0 to 1, each 0 to 255, for the canvas. */
export function rgbaOf(color: Oklch, alpha = 1): Rgba {
  const [r, g, b] = linearSrgbOf(color).map((channel) => Math.round(encode(channel) * 255));
  return [r ?? 0, g ?? 0, b ?? 0, Math.round(Math.min(1, Math.max(0, alpha)) * 255)];
}
