// The tunable numbers of the radial layout, each with the reason for its value. Distances are in
// world units; the camera decides how many pixels one unit is.

/**
 * Distance between rings, and so the least distance between two files of one directory.
 * Drawn file dots are smaller than this so neighbours never touch.
 */
export const RING_SPACING = 10;

/**
 * Ring k holds `SLOTS_PER_RING_STEP * k` slots. Slots grow in step with the ring radius, so the
 * arc between neighbours stays near `RING_SPACING` on every ring (exactly equal on ring 1, a hexagon).
 */
export const SLOTS_PER_RING_STEP = 6;

/** A hub with no files of its own still keeps this many rings clear around its centre for its drawn core. */
export const MIN_RINGS = 1;

/** Clear space kept between a sub-tree's covering circle and its neighbours and parent's files. */
export const SUBTREE_GAP = RING_SPACING;

/** Margin added to each hub's covering radius so its outline and label do not touch the neighbours. */
export const HUB_PADDING = RING_SPACING / 2;

/**
 * Each sub-directory is guaranteed this share of an equal split of the full circle, so a hub
 * with one file beside a hub with ten thousand still gets a wedge wide enough to stay clear.
 */
export const MIN_SECTOR_SHARE = 0.25;

/** The angle at which a hub's sub-directory sectors start, in radians from the positive x axis. */
export const SECTOR_START_ANGLE = 0;

/**
 * A hub draws as one collapsed node while its covering circle is smaller than this on screen,
 * in pixels. Below it the rings would be smaller than a few pixels and only cost frame time.
 */
export const COLLAPSE_BELOW_RADIUS_PX = 24;

/** On-screen radius in pixels of a collapsed node holding one file. */
export const COLLAPSED_BASE_RADIUS_PX = 3;

/** Extra radius in pixels per square root of files in a collapsed node (area grows with file count). */
export const COLLAPSED_RADIUS_PX_PER_SQRT_FILE = 0.75;

/** Collapsed nodes never draw larger than this radius in pixels, whatever their file count. */
export const COLLAPSED_MAX_RADIUS_PX = COLLAPSE_BELOW_RADIUS_PX;
