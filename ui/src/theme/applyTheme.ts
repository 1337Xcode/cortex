// Writes the tokens of the active theme onto the document as CSS custom properties, so the
// stylesheet reads tokens and never literal values. Follows the system's colour scheme and its
// reduced-motion setting as they change.
import {
  CANVAS_SIZE,
  CHROME,
  FONT_FAMILY,
  MONO_FAMILY,
  MOTION,
  OPACITY,
  SPACING,
  THEMES,
  TYPE_SIZES,
  type ThemeColors,
} from './tokens';
import { DARK_QUERY, REDUCED_MOTION_QUERY } from './media';

/** The CSS custom properties for one theme and motion setting, name to value. */
export function themeProperties(
  colors: ThemeColors,
  isReducedMotion: boolean,
): ReadonlyMap<string, string> {
  const properties = new Map<string, string>([
    ['--color-background', colors.background],
    ['--color-surface', colors.surface],
    ['--color-border', colors.border],
    ['--color-text', colors.text],
    ['--color-text-muted', colors.textMuted],
    ['--color-accent', colors.accent],
    ['--color-danger', colors.danger],
    ['--color-recent', colors.canvas.recent],
    ['--color-suspect', colors.canvas.suspect],
    ['--color-unread', colors.canvas.unread],
    ['--color-hub', colors.canvas.hub],
    ['--color-file', colors.canvas.file],
    ['--collapsed-fill-opacity', String(OPACITY.collapsedFill)],
    ['--collapsed-stroke-width', `${CANVAS_SIZE.collapsedStrokePx}px`],
    ['--font-family', FONT_FAMILY],
    ['--font-mono', MONO_FAMILY],
    ['--motion-duration', `${isReducedMotion ? MOTION.reducedDurationMs : MOTION.durationMs}ms`],
  ]);
  for (const [name, value] of Object.entries(SPACING))
    properties.set(`--space-${name}`, `${value}px`);
  for (const [name, value] of Object.entries(TYPE_SIZES))
    properties.set(`--size-${name}`, `${value}px`);
  for (const [name, value] of Object.entries(CHROME)) {
    const kebab = name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
    properties.set(`--chrome-${kebab}`, name.endsWith('Share') ? `${value * 100}vh` : `${value}px`);
  }
  return properties;
}

function apply(root: HTMLElement, isDark: boolean, isReducedMotion: boolean): void {
  const colors = isDark ? THEMES.dark : THEMES.light;
  for (const [name, value] of themeProperties(colors, isReducedMotion)) {
    root.style.setProperty(name, value);
  }
  root.style.setProperty('color-scheme', isDark ? 'dark' : 'light');
}

/** Applies the theme and motion setting that match the system now and whenever they change. */
export function followSystemTheme(root: HTMLElement): void {
  const dark = window.matchMedia(DARK_QUERY);
  const reduced = window.matchMedia(REDUCED_MOTION_QUERY);
  const update = (): void => {
    apply(root, dark.matches, reduced.matches);
  };
  update();
  dark.addEventListener('change', update);
  reduced.addEventListener('change', update);
}
