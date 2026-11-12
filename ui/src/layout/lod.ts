// Level of detail: which hubs draw open (their rings and sub-directories visible), which draw as
// one collapsed node, and which are hidden inside a collapsed ancestor. The frame then draws
// only what is large enough on screen to be read, however many files the repository has.
import {
  COLLAPSE_BELOW_RADIUS_PX,
  COLLAPSED_BASE_RADIUS_PX,
  COLLAPSED_MAX_RADIUS_PX,
  COLLAPSED_RADIUS_PX_PER_SQRT_FILE,
} from './constants';
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

/** Draw states for every hub and the on-screen radius of each collapsed node. */
export interface LevelOfDetail {
  readonly state: Uint8Array;
  /** On-screen radius in pixels for each collapsed hub; 0 for the others. */
  readonly collapsedRadiusPx: Float32Array;
}

/** Pixels per world unit at a deck.gl zoom level: each zoom step doubles the scale. */
export function pixelsPerUnit(zoom: number): number {
  return 2 ** zoom;
}

/** The radius in pixels of a collapsed node holding `fileCount` files, from one file up to the cap. */
export function collapsedRadiusPx(fileCount: number): number {
  const radius =
    COLLAPSED_BASE_RADIUS_PX + COLLAPSED_RADIUS_PX_PER_SQRT_FILE * Math.sqrt(fileCount);
  return Math.min(radius, COLLAPSED_MAX_RADIUS_PX);
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
  const collapsed = new Float32Array(hubCount);
  for (let h = 0; h < hubCount; h += 1) {
    const parent = layout.hubParent[h] ?? -1;
    if (parent >= 0 && state[parent] !== HubState.Open) {
      state[h] = HubState.Hidden;
    } else if ((layout.hubRadius[h] ?? 0) * scale < COLLAPSE_BELOW_RADIUS_PX) {
      state[h] = HubState.Collapsed;
      collapsed[h] = collapsedRadiusPx(layout.hubFileCount[h] ?? 0);
    }
  }
  return { state, collapsedRadiusPx: collapsed };
}
