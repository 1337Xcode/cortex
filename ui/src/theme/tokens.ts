// Design tokens: every colour, size, opacity and timing the map uses. Components, layers and the
// stylesheet read from here and hold no literal values of their own.
//
// Sizes sit on one 4 px grid. Spacing grows by about the golden ratio from 8 px on (8, 12, 20, 32,
// 52), rounded to the grid; type steps by its square root and by the ratio itself (13 x 1.618 is
// 21), and every line height is a grid step, so text lines line up with the boxes around them.
// Colours are written in OKLCH: a neutral ramp of twelve steps per theme, one accent hue, and the
// status hues as the accent's split complement at the accent's own lightness and chroma, so none
// shouts over another.
import type { LinkKind } from '../api/graph';
import { oklch, type Oklch } from './oklch';

/** The grid every size, gap and line height is a multiple of, in pixels. */
export const GRID_PX = 4;

/** The golden ratio, which the spacing and type scales follow. */
export const GOLDEN_RATIO = (1 + Math.sqrt(5)) / 2;

/** Spacing steps in pixels, from a hairline gap to the space between regions. */
export const SPACE = { xs: 4, sm: 8, md: 12, lg: 20, xl: 32, xxl: 52 } as const;

/** One step of the spacing scale. */
export type SpaceStep = keyof typeof SPACE;

/** One step of the type scale: its size and the grid line height it sits on, in pixels. */
export interface TypeStep {
  readonly sizePx: number;
  readonly lineHeightPx: number;
}

/**
 * Type steps. Small is the least any text on the map is drawn at; body is the panels' text; heading
 * names a section; title names the selected file.
 */
export const TYPE = {
  small: { sizePx: 12, lineHeightPx: 16 },
  body: { sizePx: 13, lineHeightPx: 20 },
  heading: { sizePx: 16, lineHeightPx: 24 },
  title: { sizePx: 21, lineHeightPx: 32 },
} as const satisfies Readonly<Record<string, TypeStep>>;

/** Font weights of the variable fonts. */
export const WEIGHT = { regular: 400, medium: 500, strong: 600 } as const;

/**
 * The two families the map ships and draws every word in: Geist for words, Geist Mono for paths,
 * line numbers, code and numbers in tables. The fallbacks only show if a font failed to load.
 */
export const FONT = {
  sans: '"Geist", system-ui, sans-serif',
  mono: '"Geist Mono", ui-monospace, monospace',
} as const;

/** The family names the fonts register under, which the page waits for before drawing labels. */
export const FONT_NAME = { sans: 'Geist', mono: 'Geist Mono' } as const;

/** Sizes of the page's chrome, in pixels, each a multiple of the grid. */
export const SIZE = {
  /** Buttons, the search box and the summary strip. */
  control: 32,
  /** The small buttons inside a control: zoom steps, a section's toggle. */
  controlCompact: 24,
  /** A list row: one body line and its padding. */
  row: 28,
  icon: 16,
  /** An icon beside the selected file's title, whose text is 21 px. */
  iconTitle: 20,
  /** The key hint beside a button's name. */
  keyHint: 16,
  /** The selected file's panel. */
  panel: 340,
  /** The overview panel on the left, and the search box above it. */
  overview: 300,
  legend: 380,
  /** The zoom percentage between its two buttons. */
  zoomValue: 48,
  /** The first-open sentence's card. */
  intro: 380,
  radius: 8,
  radiusSmall: 4,
  /** The sample beside each legend entry. */
  swatchWidth: 28,
  swatchHeight: 16,
} as const;

/**
 * Stroke widths in pixels: lines, not boxes, so they are not grid steps. The hairline is every
 * border; the focus ring is drawn outside its control.
 */
export const STROKE = {
  hairline: 1,
  focus: 2,
  focusOffset: 2,
  /** A fact's coloured edge in the panel. */
  edge: 4,
  /** A file's outline in the legend's samples, and a never-read file's thinner one. */
  mark: 1.5,
  markThin: 0.75,
} as const;

/** Layout rules of the page that are not sizes. */
export const LAYOUT = {
  /** Below this viewport width the panels become bottom sheets and the chrome compacts. */
  narrowBelow: 720,
  /** A bottom sheet's height as a share of the viewport. */
  sheetHeightShare: 0.58,
  /** The search results' and the legend's greatest height as a share of the viewport. */
  popoverHeightShare: 0.7,
} as const;

/**
 * How many rows a list shows before "Show N more", how many a page of history holds, and the most
 * rows an overview list keeps (its strongest; a long tail is for search, not a list).
 */
export const LIST = { rows: 5, historyPage: 10, most: 25 } as const;

/**
 * The receipts chart, in pixels: each plot's height, its marks (a dot of 8 px across with a 2 px
 * ring in the panel's colour, larger while it is the task explained), how far the marks' centres
 * keep from the plot's edges (the explained mark and its ring, 8 px, and 4 px clear of the focus
 * ring), and how many of the newest tasks it draws (more than this crowd a panel's width; the
 * sessions list below it holds the rest).
 */
export const CHART = {
  plotHeight: 72,
  dotRadius: 4,
  dotRing: 2,
  selectedRadius: 6,
  insetPx: SPACE.md,
  maxTasks: 60,
} as const;

/** The hue of the one accent: learned links, the selection and the focus ring. */
export const ACCENT_HUE = 290;

/** How far each status hue sits from the accent's complement, in degrees. */
export const SPLIT_DEGREES = 35;

/** The hue of red, kept for an error or a broken anchor. */
export const DANGER_HUE = 25;

/** A hue turned into 0 to 360 degrees. */
function wrapHue(degrees: number): number {
  return ((degrees % 360) + 360) % 360;
}

/** The status hues: the accent's split complement (green for recent, amber for suspect). */
export const STATUS_HUE = {
  recent: wrapHue(ACCENT_HUE + 180 + SPLIT_DEGREES),
  suspect: wrapHue(ACCENT_HUE + 180 - SPLIT_DEGREES),
} as const;

/** The hue and chroma of the neutral ramp: a cool grey. */
const NEUTRAL_HUE = 260;

/** Lightness of each step of the dark theme's neutral ramp, ground to brightest text. */
const DARK_RAMP_L = [0.17, 0.2, 0.235, 0.27, 0.31, 0.36, 0.43, 0.52, 0.62, 0.7, 0.79, 0.95];

/** Lightness of each step of the light theme's neutral ramp, brightest surface to darkest text. */
const LIGHT_RAMP_L = [1, 0.975, 0.955, 0.925, 0.89, 0.85, 0.78, 0.62, 0.56, 0.5, 0.46, 0.22];

/** A ramp of twelve neutral steps at one chroma. */
function neutralRamp(lightness: readonly number[], chroma: number): readonly Oklch[] {
  return lightness.map((l) => oklch(l, l === 1 ? 0 : chroma, NEUTRAL_HUE));
}

/** The twelve neutral steps of each theme. */
export const NEUTRAL = {
  dark: neutralRamp(DARK_RAMP_L, 0.012),
  light: neutralRamp(LIGHT_RAMP_L, 0.004),
} as const;

/** The lightness and chroma the accent and every status hue share in each theme. */
export const SIGNAL = { dark: { l: 0.77, c: 0.12 }, light: { l: 0.52, c: 0.11 } } as const;

/**
 * Lightness of each step of the views' sequential scale, weakest to strongest: the accent's hue
 * at its chroma, in equal steps of 0.08, from the dimmest that still stands 3:1 off the ground up
 * to the accent's own lightness (dark) or down past it (light).
 */
const SCALE_L = { dark: [0.53, 0.61, 0.69, 0.77], light: [0.64, 0.56, 0.48, 0.4] } as const;

/** The views' sequential scale: four colours, weakest to strongest. */
export type ScaleColors = readonly [Oklch, Oklch, Oklch, Oklch];

/** The agent families the live markers tell apart; every other agent shares the neutral `other`. */
export const AGENT_FAMILIES = ['claude', 'codex', 'cursor', 'gemini', 'other'] as const;

/** One of `AGENT_FAMILIES`. */
export type AgentFamily = (typeof AGENT_FAMILIES)[number];

/**
 * The hue of each named agent family, at the signal lightness: rose, cyan, blue and lime, each at
 * least 35 degrees from the accent (290), the status hues (75, 145) and red (25).
 */
export const AGENT_HUE: Readonly<Record<Exclude<AgentFamily, 'other'>, number>> = {
  claude: 345,
  codex: 200,
  cursor: 250,
  gemini: 110,
};

/**
 * The agent families' chroma per theme, the same for every family: the signal chroma in the dark
 * theme, and lower in the light one, where cyan at the signal lightness has no more in sRGB.
 */
const AGENT_CHROMA = { dark: SIGNAL.dark.c, light: 0.085 } as const;

/** The colours of the canvas: neutral by default, colour only where it means something. */
export interface CanvasColors {
  /** The faint dot grid behind the graph. */
  readonly gridDot: Oklch;
  /** Folder squares and a collapsed folder's outline. */
  readonly hub: Oklch;
  /** A collapsed folder's fill. */
  readonly collapsedFill: Oklch;
  /** The faint dashed line from a folder to its parent, and a file's spoke to its folder. */
  readonly hubLink: Oklch;
  /** A file's outline. */
  readonly file: Oklch;
  /** A file no agent has read yet: a dimmed outline. */
  readonly unread: Oklch;
  /** A file read or edited by an agent in the last 24 hours. */
  readonly recent: Oklch;
  /** A file with a suspect fact. */
  readonly suspect: Oklch;
  /** Folder and file names drawn on the canvas. */
  readonly label: Oklch;
  /** A file the Fog view lights up: one an agent has read. */
  readonly lit: Oklch;
  /** The views' sequential scale, weakest to strongest (the accent's hue at its chroma). */
  readonly scale: ScaleColors;
  /** A file's fact badge; a badge with a suspect fact is drawn in `suspect`. */
  readonly badge: Oklch;
}

/** The colours of one theme. Text pairs meet WCAG AA (4.5:1), marks 3:1 (see the tests). */
export interface ThemeColors {
  /** The ground under the canvas. */
  readonly background: Oklch;
  /** Panels, the search box and every control. */
  readonly surface: Oklch;
  /** A control under the pointer, an open section's header. */
  readonly raised: Oklch;
  readonly border: Oklch;
  readonly text: Oklch;
  readonly textMuted: Oklch;
  /** The one accent: learned links, the selection and the focus ring. */
  readonly accent: Oklch;
  /** Text drawn on the accent. */
  readonly onAccent: Oklch;
  /** An error. */
  readonly danger: Oklch;
  readonly canvas: CanvasColors;
  /** Each agent family's live marker, trail and name. */
  readonly agents: Readonly<Record<AgentFamily, Oklch>>;
}

/** The accent and status colours of a theme at its signal lightness and chroma. */
function signals(theme: keyof typeof SIGNAL): {
  accent: Oklch;
  recent: Oklch;
  suspect: Oklch;
  danger: Oklch;
} {
  const { l, c } = SIGNAL[theme];
  return {
    accent: oklch(l, c, ACCENT_HUE),
    recent: oklch(l, c, STATUS_HUE.recent),
    suspect: oklch(l, c, STATUS_HUE.suspect),
    danger: oklch(l, c, DANGER_HUE),
  };
}

/** The views' sequential scale of a theme. */
function scaleOf(theme: keyof typeof SIGNAL): ScaleColors {
  const [a, b, c, d] = SCALE_L[theme];
  const at = (l: number): Oklch => oklch(l, SIGNAL[theme].c, ACCENT_HUE);
  return [at(a), at(b), at(c), at(d)];
}

/** The agent family colours of a theme; `other` is the neutral it is given. */
function agentsOf(theme: keyof typeof SIGNAL, other: Oklch): Record<AgentFamily, Oklch> {
  const { l } = SIGNAL[theme];
  const c = AGENT_CHROMA[theme];
  return {
    claude: oklch(l, c, AGENT_HUE.claude),
    codex: oklch(l, c, AGENT_HUE.codex),
    cursor: oklch(l, c, AGENT_HUE.cursor),
    gemini: oklch(l, c, AGENT_HUE.gemini),
    other,
  };
}

/** Step `index` (1 to 12) of a ramp. */
function step(ramp: readonly Oklch[], index: number): Oklch {
  const color = ramp[index - 1];
  if (color === undefined) throw new Error(`the neutral ramp has no step ${index}`);
  return color;
}

/** The dark theme (the default look): a near-black ground, cool greys and a light violet accent. */
export const DARK_COLORS: ThemeColors = (() => {
  const n = NEUTRAL.dark;
  const { accent, recent, suspect, danger } = signals('dark');
  return {
    background: step(n, 1),
    surface: step(n, 2),
    raised: step(n, 3),
    border: step(n, 5),
    text: step(n, 12),
    textMuted: step(n, 11),
    accent,
    onAccent: step(n, 1),
    danger,
    canvas: {
      gridDot: step(n, 4),
      hub: step(n, 10),
      collapsedFill: step(n, 4),
      hubLink: step(n, 7),
      file: step(n, 10),
      unread: step(n, 8),
      recent,
      suspect,
      label: step(n, 11),
      lit: step(n, 12),
      scale: scaleOf('dark'),
      badge: step(n, 11),
    },
    agents: agentsOf('dark', step(n, 11)),
  };
})();

/** The light theme: a near-white ground, the same greys darkened and a deep violet accent. */
export const LIGHT_COLORS: ThemeColors = (() => {
  const n = NEUTRAL.light;
  const { accent, recent, suspect, danger } = signals('light');
  return {
    background: step(n, 2),
    surface: step(n, 1),
    raised: step(n, 3),
    border: step(n, 5),
    text: step(n, 12),
    textMuted: step(n, 11),
    accent,
    onAccent: step(n, 1),
    danger,
    canvas: {
      gridDot: step(n, 5),
      hub: step(n, 10),
      collapsedFill: step(n, 4),
      hubLink: step(n, 7),
      file: step(n, 10),
      unread: step(n, 8),
      recent,
      suspect,
      label: step(n, 11),
      lit: step(n, 12),
      scale: scaleOf('light'),
      badge: step(n, 11),
    },
    agents: agentsOf('light', step(n, 11)),
  };
})();

/** The page's colour roles, in the order the checks list them. */
const PAGE_ROLES = [
  'background',
  'surface',
  'raised',
  'border',
  'text',
  'textMuted',
  'accent',
  'onAccent',
  'danger',
] as const;

/** The canvas's colour roles. */
const CANVAS_ROLES = [
  'gridDot',
  'hub',
  'collapsedFill',
  'hubLink',
  'file',
  'unread',
  'recent',
  'suspect',
  'label',
  'lit',
  'badge',
] as const;

/**
 * Every colour of a theme by role name: the canvas roles prefixed `canvas.`, the scale's steps
 * `canvas.scale.1` to `canvas.scale.4`, the agent families `agent.`.
 */
export function colorRoles(colors: ThemeColors): [string, Oklch][] {
  return [
    ...PAGE_ROLES.map((role): [string, Oklch] => [role, colors[role]]),
    ...CANVAS_ROLES.map((role): [string, Oklch] => [`canvas.${role}`, colors.canvas[role]]),
    ...colors.canvas.scale.map((color, i): [string, Oklch] => [`canvas.scale.${i + 1}`, color]),
    ...AGENT_FAMILIES.map((family): [string, Oklch] => [`agent.${family}`, colors.agents[family]]),
  ];
}

/** Both themes by name; the page picks one from the system preference. */
export const THEMES = { light: LIGHT_COLORS, dark: DARK_COLORS } as const;

/** One theme's name. */
export type ThemeName = keyof typeof THEMES;

/**
 * Dash pattern of each link kind in pixels, dash then gap; empty is solid. Kinds differ by
 * pattern, so colour is never the only cue: changed together solid, used together dashed, named
 * in a work order dotted.
 */
export const LINK_DASH_PX: Readonly<Record<LinkKind, readonly [number, number] | readonly []>> = {
  co_change: [],
  used_together: [6, 4],
  work_order: [1.5, 3.5],
};

/** Dash pattern of the folder-to-parent lines, in pixels. */
export const HUB_LINK_DASH_PX = [3, 4] as const;

/** Opacities, from 0 to 1. */
export const OPACITY = {
  /** A learned link at weight 0; weight 1 draws at `linkMax`, linearly between. */
  linkMin: 0.2,
  linkMax: 0.6,
  /** A collapsed folder's disc fill, in the folder colour. */
  collapsedFill: 0.3,
  /** A link past the strongest few of its kind while the frame is dense. */
  linkFaint: 0.05,
  /** A link of the hovered or selected file. */
  linkFocused: 0.95,
  /** Anything outside the hovered or selected file's neighbourhood, as a share of its alpha. */
  dimmed: 0.18,
  hubLink: 0.6,
  /** The spoke from a file to its folder. */
  spoke: 0.28,
  grid: 1,
  /** A label outside the focus. */
  labelDimmed: 0.4,
  /** The backing behind canvas labels, so they read over lines. */
  labelBacking: 0.82,
  /**
   * How far a tinting view fills a file towards its tone's colour, from the ground: enough to
   * read the scale at a glance, little enough that the outline and the recent dot stay clear.
   */
  toneFill: 0.3,
  /** A live marker's diamond fill, in its agent's colour, so the file beneath still shows. */
  markerFill: 0.16,
  /** A trail's newest step; each older one fades evenly down to `trailOldest`. */
  trailNewest: 0.5,
  trailOldest: 0.1,
} as const;

/** Sizes on the canvas. Units say which are pixels and which are world units of the layout. */
export const CANVAS_SIZE = {
  /**
   * A file circle's radius in world units grows with the square root of its lines, clamped.
   * The cap keeps neighbours apart: the closest two files of a folder are one ring spacing (10)
   * apart, and two circles of 2.8 with a suspect ring and the thick selection outline still leave
   * a gap at every zoom where single files are drawn (see the tests).
   */
  fileRadiusMin: 1.2,
  fileRadiusMax: 2.8,
  fileRadiusPerSqrtLine: 0.07,
  /** Circles never draw smaller than this many pixels, so far-out files stay visible. */
  fileRadiusMinPx: 0.8,
  fileStrokePx: 1.1,
  fileStrokeFocusedPx: 2.2,
  /** A never-read file's thinner outline, its second channel beside the dim colour. */
  fileStrokeUnreadPx: 0.6,
  /** The suspect ring sits this far outside the file circle, in world units. */
  suspectRingGap: 0.6,
  /** The recent dot's radius as a share of the file circle's. */
  recentDotShare: 0.45,
  /**
   * Files draw one by one once neighbouring rings are at least this many pixels apart; closer
   * than that, each open folder draws its own files as one disc.
   */
  fileMinRingSpacingPx: 8,
  /**
   * Half the side of a folder's square, in pixels: a 6 px square. Off the grid on purpose: an 8 px
   * square swallowed the smallest folders' discs (5 px across) and read as a cluster of blocks.
   */
  hubHalfSidePx: 3,
  collapsedStrokePx: 1.5,
  /** A collapsed folder's radius as a share of the circle its sub-tree covers. */
  collapsedShareOfReach: 0.8,
  /** Collapsed folders never draw smaller than this many pixels. */
  collapsedMinPx: 2.5,
  linkWidthPx: 1.25,
  linkFocusedWidthPx: 2,
  hubLinkWidthPx: 1,
  spokeWidthPx: 0.75,
  /** Spokes are drawn only in folders whose own files fit on this many rings (18 files). */
  spokeMaxRings: 2,
  gridPitchPx: 16,
  gridDotRadiusPx: 0.75,
  /** How near the pointer must come to a mark or a link to point at it, in pixels. */
  pickRadiusPx: 4,
  /** A live marker's diamond clears its file's circle by this many pixels. */
  markerGapPx: 3,
  /** A live marker's diamond is never smaller than this, half its diagonal in pixels. */
  markerMinPx: 7,
  /** Each further session on the same spot draws its diamond this many pixels larger. */
  markerStackPx: 4,
  markerStrokePx: 2,
  trailWidthPx: 1.25,
  /** The dot on each earlier file of a session's trail, in pixels. */
  trailStopPx: 2,
  /** A fact badge on a file's rim, in pixels, with a ground-coloured edge to part it from the rim. */
  badgeRadiusPx: 3,
  badgeEdgePx: 1,
} as const;

/** Labels drawn on the canvas, in the same family and at the same smallest size as the page. */
export const LABEL = {
  hubSizePx: TYPE.small.sizePx,
  fileSizePx: TYPE.small.sizePx,
  hubWeight: WEIGHT.strong,
  fileWeight: WEIGHT.regular,
  /** A live marker's name: its agent and short session id, in the agent's colour. */
  markerWeight: WEIGHT.strong,
  /**
   * The most live markers named over other nodes in one frame (zoomed out, on folders' discs);
   * more would bury the folders' names, and every session is named in its row and on hover.
   */
  markerOverMax: 6,
  /** Space between a node and its label, in pixels. */
  gapPx: SPACE.xs,
  /**
   * Clear space kept around every label so neighbours never touch, in pixels: half a grid step, as
   * a whole step leaves the hovered file nameless where its folder's square sits close beside it.
   */
  paddingPx: SPACE.xs / 2,
  /** The backing's padding around the text, in pixels. */
  backingPaddingPx: [SPACE.xs, 0] as const,
  /** Files get names once the camera shows at least this many pixels per world unit (rings 20 px apart). */
  fileMinPixelsPerUnit: 2,
  /** The most labels in one frame; beyond this the frame costs more than the labels help. */
  maxCount: 260,
  /** A collapsed folder is named only when it draws at least this many pixels across. */
  collapsedMinLabelPx: 12,
  /** Line height as a share of the type size, for collision boxes. */
  lineHeight: TYPE.small.lineHeightPx / TYPE.small.sizePx,
} as const;

/**
 * When more than `denseAbove` drawn links have an end near the screen, only the strongest
 * `strongestPerKind` of each kind (and the hovered or selected file's) keep their strength. While
 * files are too small to draw one by one (the overview), only the `overviewStrongest` strongest
 * links of the whole frame are drawn at all, until the person zooms in or selects a file.
 */
export const LINK_DENSITY = { denseAbove: 40, strongestPerKind: 6, overviewStrongest: 12 } as const;

/** Camera limits and steps. */
export const CAMERA = {
  /** Each − or + step scales by this factor. */
  zoomStepFactor: 1.3,
  minZoom: -14,
  maxZoom: 5,
  /** Space kept around the graph when fitting it, in pixels. */
  fitPaddingPx: SPACE.xxl,
  /**
   * Fitting never zooms in past this level (8 pixels per world unit): a small repository fills
   * the view, but its files never draw larger than 24 pixels across.
   */
  maxFitZoom: 3,
  /** Flying to a file zooms in to at least this zoom level (4 pixels per world unit). */
  focusZoom: 2,
  /** One arrow key pans by this many pixels. */
  panStepPx: 80,
} as const;

/** Motion: 150 ms eased, none when the person asks for reduced motion. */
export const MOTION = { durationMs: 150, reducedDurationMs: 0 } as const;

/** How long the pointer rests on something before its explanation shows, in milliseconds. */
export const TOOLTIP_DELAY_MS = 300;

/** A file read or edited by an agent within this many milliseconds counts as recent (24 hours). */
export const RECENT_WINDOW_MS = 24 * 60 * 60 * 1000;

/** The link strength threshold the map opens with, from 0 to 1. */
export const DEFAULT_MIN_STRENGTH = 0.25;

/** How long the search box waits after the last key before asking, in milliseconds. */
export const SEARCH_DEBOUNCE_MS = 120;

/** While the map has no files, the page asks the daemon again this often, in milliseconds. */
export const EMPTY_RECHECK_MS = 5000;

/**
 * The live view: a lost connection is tried again after `reconnectFirstMs`, doubling each time up
 * to `reconnectMaxMs`; a session id is shown by its first `shortIdChars` characters; a marker's
 * explanation names at most `namedBefore` of the files the session worked on before its newest.
 */
export const LIVE = {
  reconnectFirstMs: 1000,
  reconnectMaxMs: 30_000,
  shortIdChars: 4,
  namedBefore: 3,
} as const;
