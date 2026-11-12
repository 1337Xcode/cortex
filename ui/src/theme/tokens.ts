// Design tokens: every colour, spacing step and type size the map uses. Components read from
// here and hold no literal values of their own.
import type { LinkKind } from '../api/graph';

/** The colours of one theme. Text pairs meet WCAG AA (4.5:1), link colours 3:1 (see the tests). */
export interface ThemeColors {
  readonly background: string;
  readonly surface: string;
  readonly border: string;
  readonly text: string;
  readonly textMuted: string;
  readonly accent: string;
  readonly danger: string;
  /** Link colours, one per kind: blue, orange and green hues taken from the Okabe-Ito palette, which stays distinguishable under the common colour-vision deficiencies. */
  readonly link: Readonly<Record<LinkKind, string>>;
}

/** Light theme. Orange is the Okabe-Ito vermillion and green a darker bluish green, so each holds 3:1 on white. */
export const LIGHT_COLORS: ThemeColors = {
  background: '#ffffff',
  surface: '#f4f6f8',
  border: '#c4ccd6',
  text: '#1a1f27',
  textMuted: '#4a5361',
  accent: '#0b5cad',
  danger: '#b3261e',
  link: {
    co_change: '#0072b2',
    used_together: '#d55e00',
    work_order: '#007a5a',
  },
};

/** Dark theme. The lighter blue (Okabe-Ito sky blue), orange and a lighter green keep 3:1 on the dark background. */
export const DARK_COLORS: ThemeColors = {
  background: '#12161c',
  surface: '#1b212a',
  border: '#3a4452',
  text: '#e8ecf1',
  textMuted: '#a9b3c1',
  accent: '#6cb2f0',
  danger: '#ff8a80',
  link: {
    co_change: '#56b4e9',
    used_together: '#e69f00',
    work_order: '#2fc79a',
  },
};

/** Both themes by name; the page picks one from the system preference. */
export const THEMES = { light: LIGHT_COLORS, dark: DARK_COLORS } as const;

/** Spacing scale in pixels. */
export const SPACING = { xs: 4, sm: 8, md: 16, lg: 24, xl: 32 } as const;

/** Type sizes in pixels. */
export const TYPE_SIZES = { small: 12, body: 14, heading: 18, title: 24 } as const;

/** The system font stack; the map ships no web fonts. */
export const FONT_FAMILY =
  'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';
