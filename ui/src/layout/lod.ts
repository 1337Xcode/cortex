// Level of detail: which hubs draw open (their rings and sub-directories visible), which draw as
// one collapsed node, and which are hidden inside a collapsed ancestor. The frame then draws
// only what is large enough on screen to be read, however many files the repository has.
import { COLLAPSE_BELOW_RADIUS_PX } from './constants';
import type { MapLayout } from './radial';

/** The draw state of a hub at one zoom level. */
export const HubState = {
  /** Draw the hub's own files and its sub-directory hubs. */
  Open: 0,
  /** Draw the whole sub-tree as one node. */
  Collapsed: 1,
  /** Draw nothing: an ancestor is collapsed. */
  Hidden: 2,
} as const;

/** One of the `HubState` values. */
export type HubStateValue = (typeof HubState)[keyof typeof HubState];

/**
 * Draw states for every hub. How big a collapsed node draws is the map's choice (see
 * `map/hubEncoding.ts`): a share of the circle its sub-tree covers, so siblings never overlap.
 */
export interface LevelOfDetail {
  readonly state: Uint8Array;
}

/** Pixels per world unit at a deck.gl zoom level: each zoom step doubles the scale. */
export function pixelsPerUnit(zoom: number): number {
  return 2 ** zoom;
}

/**
 * Decides every hub's draw state at `zoom`. A hub is open while its covering circle is at least
 * `COLLAPSE_BELOW_RADIUS_PX` on screen and its parent is open. One pass in pre-order, since a
 * parent always comes before its children.
 */
export function levelOfDetail(layout: MapLayout, zoom: number): LevelOfDetail {
  const scale = pixelsPerUnit(zoom);
  const hubCount = layout.hubParent.length;
  const state = new Uint8Array(hubCount);
  for (let h = 0; h < hubCount; h += 1) {
    const parent = layout.hubParent[h] ?? -1;
    if (parent >= 0 && state[parent] !== HubState.Open) {
      state[h] = HubState.Hidden;
    } else if ((layout.hubRadius[h] ?? 0) * scale < COLLAPSE_BELOW_RADIUS_PX) {
      state[h] = HubState.Collapsed;
    }
  }
  return { state };
}
