// Writes the tokens of the active theme onto the document as CSS custom properties, so the
// stylesheet and the components read tokens and never literal values.
import { FONT_FAMILY, SPACING, THEMES, TYPE_SIZES, type ThemeColors } from './tokens';

/** The CSS custom properties for one theme, name to value. */
export function themeProperties(colors: ThemeColors): ReadonlyMap<string, string> {
  const properties = new Map<string, string>([
    ['--color-background', colors.background],
    ['--color-surface', colors.surface],
    ['--color-border', colors.border],
    ['--color-text', colors.text],
    ['--color-text-muted', colors.textMuted],
    ['--color-accent', colors.accent],
    ['--color-danger', colors.danger],
    ['--font-family', FONT_FAMILY],
  ]);
  for (const [name, value] of Object.entries(SPACING))
    properties.set(`--space-${name}`, `${value}px`);
  for (const [name, value] of Object.entries(TYPE_SIZES))
    properties.set(`--size-${name}`, `${value}px`);
  return properties;
}

function apply(root: HTMLElement, isDark: boolean): void {
  const colors = isDark ? THEMES.dark : THEMES.light;
  for (const [name, value] of themeProperties(colors)) root.style.setProperty(name, value);
  root.style.setProperty('color-scheme', isDark ? 'dark' : 'light');
}

/** Applies the theme that matches the system preference now and whenever the preference changes. */
export function followSystemTheme(root: HTMLElement): void {
  const query = window.matchMedia('(prefers-color-scheme: dark)');
  apply(root, query.matches);
  query.addEventListener('change', (event) => {
    apply(root, event.matches);
  });
}
