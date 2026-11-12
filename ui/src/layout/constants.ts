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

/**
 * Clear space kept between a sub-tree's covering circle and its sector's edges and its parent's
 * own rings. With the padding inside every covering circle, a sub-tree's files stay at least
 * gap + padding = 10 (one ring spacing) from its parent's files, and twice that from a sibling's.
 */
export const SUBTREE_GAP = RING_SPACING / 2;

/** Margin added to each hub's covering radius so its outline and label do not touch the neighbours. */
export const HUB_PADDING = RING_SPACING / 2;

/** The angle at which a hub's sub-directory sectors start, in radians from the positive x axis. */
export const SECTOR_START_ANGLE = 0;

/**
 * A hub draws as one collapsed node while its covering circle is smaller than this on screen,
 * in pixels. Below it the rings would be smaller than a few pixels and only cost frame time.
 */
export const COLLAPSE_BELOW_RADIUS_PX = 24;
