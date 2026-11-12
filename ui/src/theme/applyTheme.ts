// Writes the tokens of the active theme onto the document as CSS custom properties, so the
// stylesheet and the components read tokens and never literal values. Colours take the names the
// shadcn/ui components read (`--background`, `--primary`, `--muted-foreground`, ...), in OKLCH, so
// the components and the canvas draw from one table. Follows the system's colour scheme and its
// reduced-motion setting as they change.
import { cssOf } from './oklch';
import {
  AGENT_FAMILIES,
  FONT,
  LAYOUT,
  MOTION,
  OPACITY,
  SIZE,
  SPACE,
  STROKE,
  THEMES,
  TYPE,
  WEIGHT,
  type ThemeColors,
} from './tokens';
import { DARK_QUERY, REDUCED_MOTION_QUERY } from './media';

/** A camelCase token name in kebab case: `controlCompact` is `control-compact`. */
function kebab(name: string): string {
  return name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`);
}

/** The colour properties of one theme, by the names the components read. */
function colorProperties(colors: ThemeColors): [string, string][] {
  const { canvas } = colors;
  const roles: [string, string][] = [
    ['background', cssOf(colors.background)],
    ['foreground', cssOf(colors.text)],
    ['card', cssOf(colors.surface)],
    ['card-foreground', cssOf(colors.text)],
    ['popover', cssOf(colors.surface)],
    ['popover-foreground', cssOf(colors.text)],
    ['primary', cssOf(colors.accent)],
    ['primary-foreground', cssOf(colors.onAccent)],
    ['secondary', cssOf(colors.raised)],
    ['secondary-foreground', cssOf(colors.text)],
    ['muted', cssOf(colors.raised)],
    ['muted-foreground', cssOf(colors.textMuted)],
    ['accent', cssOf(colors.raised)],
    ['accent-foreground', cssOf(colors.text)],
    ['destructive', cssOf(colors.danger)],
    ['border', cssOf(colors.border)],
    ['input', cssOf(colors.border)],
    ['ring', cssOf(colors.accent)],
    ['recent', cssOf(canvas.recent)],
    ['suspect', cssOf(canvas.suspect)],
    ['unread', cssOf(canvas.unread)],
    ['hub', cssOf(canvas.hub)],
    ['file', cssOf(canvas.file)],
    ['lit', cssOf(canvas.lit)],
    ['badge', cssOf(canvas.badge)],
    ...canvas.scale.map((color, i): [string, string] => [`scale-${i + 1}`, cssOf(color)]),
    ...AGENT_FAMILIES.map((family): [string, string] => [
      `agent-${family}`,
      cssOf(colors.agents[family]),
    ]),
  ];
  return roles.map(([name, value]) => [`--${name}`, value]);
}

/** The size, type, stroke and font properties, which both themes share. */
function scaleProperties(): [string, string][] {
  const properties: [string, string][] = [];
  for (const [name, value] of Object.entries(SPACE))
    properties.push([`--space-${name}`, `${value}px`]);
  for (const [name, value] of Object.entries(SIZE)) {
    properties.push([`--size-${kebab(name)}`, `${value}px`]);
  }
  for (const [name, value] of Object.entries(STROKE)) {
    properties.push([`--stroke-${kebab(name)}`, `${value}px`]);
  }
  for (const [name, value] of Object.entries(WEIGHT))
    properties.push([`--weight-${name}`, String(value)]);
  for (const [name, step] of Object.entries(TYPE)) {
    properties.push([`--type-${name}-size`, `${step.sizePx}px`]);
    properties.push([`--type-${name}-line`, `${step.lineHeightPx}px`]);
  }
  properties.push(
    ['--font-family-sans', FONT.sans],
    ['--font-family-mono', FONT.mono],
    ['--sheet-height', `${LAYOUT.sheetHeightShare * 100}vh`],
    ['--popover-height', `${LAYOUT.popoverHeightShare * 100}vh`],
    ['--collapsed-fill-opacity', String(OPACITY.collapsedFill)],
    ['--tone-fill-opacity', String(OPACITY.toneFill)],
  );
  return properties;
}

/** The CSS custom properties for one theme and motion setting, name to value. */
export function themeProperties(
  colors: ThemeColors,
  isReducedMotion: boolean,
): ReadonlyMap<string, string> {
  const duration = isReducedMotion ? MOTION.reducedDurationMs : MOTION.durationMs;
  return new Map([
    ...colorProperties(colors),
    ...scaleProperties(),
    ['--motion-duration', `${duration}ms`],
  ]);
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
