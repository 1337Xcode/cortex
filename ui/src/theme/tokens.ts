// Design tokens: every colour, size, opacity and timing the map uses. Components, layers and the
// stylesheet read from here and hold no literal values of their own.
import type { LinkKind } from '../api/graph';

/** The colours of the canvas: neutral by default, colour only where it means something. */
export interface CanvasColors {
  /** The faint dot grid behind the graph. */
  readonly gridDot: string;
  /** Folder hubs: the small square and the collapsed node's outline. */
  readonly hub: string;
  /** A collapsed hub's fill. */
  readonly collapsedFill: string;
  /** The faint dashed line from a hub to its parent, and a file's spoke to its hub. */
  readonly hubLink: string;
  /** A file's outline. */
  readonly file: string;
  /** A file no agent has read yet (the Fog). */
  readonly unread: string;
  /** A file read or edited by an agent in the last 24 hours. */
  readonly recent: string;
  /** A file with a suspect fact. */
  readonly suspect: string;
  /** Hub and file names drawn on the canvas. */
  readonly label: string;
}

/** The colours of one theme. Text pairs meet WCAG AA (4.5:1), graphics 3:1 (see the tests). */
export interface ThemeColors {
  readonly background: string;
  readonly surface: string;
  readonly border: string;
  readonly text: string;
  readonly textMuted: string;
  /** The one accent: learned links, the selection and the focus ring. */
  readonly accent: string;
  readonly danger: string;
  readonly canvas: CanvasColors;
}

/** Light theme: a near-white ground, slate neutrals and a deep violet accent. */
export const LIGHT_COLORS: ThemeColors = {
  background: '#f6f7f9',
  surface: '#ffffff',
  border: '#ccd3dc',
  text: '#161b22',
  textMuted: '#4a5361',
  accent: '#6d28d9',
  danger: '#b3261e',
  canvas: {
    gridDot: '#d3d9e0',
    hub: '#3f4854',
    collapsedFill: '#dde2e8',
    hubLink: '#8f99a6',
    file: '#5c6674',
    unread: '#b9c0ca',
    recent: '#15803d',
    suspect: '#b45309',
    label: '#2a313b',
  },
};

/** Dark theme (the default look): a near-black ground, grey neutrals and a light violet accent. */
export const DARK_COLORS: ThemeColors = {
  background: '#121418',
  surface: '#1a1e24',
  border: '#323944',
  text: '#e6e9ee',
  textMuted: '#9ea7b5',
  accent: '#a98bfa',
  danger: '#ff8a80',
  canvas: {
    gridDot: '#2a2f37',
    hub: '#8a93a1',
    collapsedFill: '#2b313a',
    hubLink: '#555e6b',
    file: '#8e98a7',
    unread: '#454c57',
    recent: '#4ade80',
    suspect: '#fbbf24',
    label: '#c8ced7',
  },
};

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

/** Dash pattern of the hub-to-parent lines, in pixels. */
export const HUB_LINK_DASH_PX = [3, 4] as const;

/** Opacities, from 0 to 1. */
export const OPACITY = {
  /** A learned link at weight 0; weight 1 draws at `linkMax`, linearly between. */
  linkMin: 0.2,
  linkMax: 0.6,
  /** A collapsed folder's disc fill, in the hub colour. */
  collapsedFill: 0.3,
  /** A link past the strongest few of its kind while the frame is dense. */
  linkFaint: 0.05,
  /** A link of the hovered or selected file. */
  linkFocused: 0.95,
  /** Anything outside the hovered or selected file's neighbourhood, as a share of its alpha. */
  dimmed: 0.18,
  hubLink: 0.6,
  /** The spoke from a file to its hub. */
  spoke: 0.28,
  grid: 1,
  /** A label outside the focus. */
  labelDimmed: 0.4,
  /** The backing behind canvas labels, so they read over lines. */
  labelBacking: 0.82,
} as const;

/** Sizes on the canvas. Units say which are pixels and which are world units of the layout. */
export const CANVAS_SIZE = {
  /**
   * A file circle's radius in world units grows with the square root of its lines, clamped.
   * The cap keeps neighbours apart: the closest two files of a folder are one ring spacing (10)
   * apart, and two circles of 2.8 with a suspect ring and the thick selection outline still leave
   * a gap at every zoom
   * where single files are drawn (see the tests).
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
  /** Half the side of a hub's square, in pixels. */
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
} as const;

/** Labels drawn on the canvas. */
export const LABEL = {
  hubSizePx: 12,
  fileSizePx: 11,
  hubWeight: 600,
  fileWeight: 400,
  /** Space between a node and its label, in pixels. */
  gapPx: 4,
  /** Clear space kept around every label so neighbours never touch, in pixels. */
  paddingPx: 2,
  /** The backing's padding around the text, in pixels. */
  backingPaddingPx: [3, 1] as const,
  /** Files get names once the camera shows at least this many pixels per world unit (rings 20 px apart). */
  fileMinPixelsPerUnit: 2,
  /** The most labels in one frame; beyond this the frame costs more than the labels help. */
  maxCount: 260,
  /** Line height as a share of the type size, for collision boxes. */
  lineHeight: 1.25,
} as const;

/**
 * When more than `denseAbove` drawn links have an end near the screen, only the strongest
 * `strongestPerKind` of each kind (and the hovered or selected file's) keep their strength.
 */
export const LINK_DENSITY = { denseAbove: 40, strongestPerKind: 6 } as const;

/** Camera limits and steps. */
export const CAMERA = {
  /** Each − or + step scales by this factor. */
  zoomStepFactor: 1.3,
  minZoom: -14,
  maxZoom: 5,
  /** Space kept around the graph when fitting it, in pixels. */
  fitPaddingPx: 48,
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

/** A file read or edited by an agent within this many milliseconds counts as recent (24 hours). */
export const RECENT_WINDOW_MS = 24 * 60 * 60 * 1000;

/** The link strength threshold the map opens with, from 0 to 1. */
export const DEFAULT_MIN_STRENGTH = 0.25;

/** How long the search box waits after the last key before asking, in milliseconds. */
export const SEARCH_DEBOUNCE_MS = 120;

/** The side panel lists at most this many links per kind. */
export const PANEL_LINKS_PER_KIND = 5;

/** Spacing scale in pixels. */
export const SPACING = { xs: 4, sm: 8, md: 16, lg: 24, xl: 32 } as const;

/** Type sizes in pixels. */
export const TYPE_SIZES = { small: 12, body: 14, heading: 16, title: 20 } as const;

/** Sizes of the page's chrome in pixels. */
export const CHROME = {
  panelWidth: 380,
  radius: 8,
  pillRadius: 999,
  controlHeight: 32,
  focusRingWidth: 2,
  focusRingOffset: 2,
  borderWidth: 1,
  /** Below this viewport width the side panel becomes a bottom sheet and the chrome compacts. */
  narrowBelow: 720,
  /** The bottom sheet's height as a share of the viewport. */
  sheetHeightShare: 0.58,
  searchWidth: 420,
  /** The search results' and the legend's greatest height as a share of the viewport. */
  popoverHeightShare: 0.6,
  legendWidth: 300,
  zoomStepSize: 22,
  keyHintHeight: 16,
  chevronSize: 6,
  /** The swatch beside each legend entry. */
  swatchWidth: 28,
  swatchHeight: 8,
  swatchTallHeight: 14,
  /**
   * A file sample's outline in the legend, and a never-read file's thinner one: the canvas's two
   * widths scaled up together, so the small sample stays readable and the difference shows.
   */
  swatchFileStrokePx: 1.5,
  swatchUnreadStrokePx: 0.9,
  /** The coloured edge of a fact in the side panel. */
  factEdge: 3,
} as const;

/** The system font stack; the map ships no web fonts. */
export const FONT_FAMILY =
  'system-ui, -apple-system, "Segoe UI", Roboto, "Helvetica Neue", Arial, sans-serif';

/** The monospace stack for paths. */
export const MONO_FAMILY = 'ui-monospace, "Cascadia Mono", "SF Mono", Menlo, Consolas, monospace';
