// Turns the level of detail into what one frame draws: the open hubs, the collapsed hubs standing
// in for their sub-trees, and for every file the node it is drawn as. A file draws itself when its
// hub is open and the rings are far enough apart on screen to tell files apart; while they are
// closer, each open hub draws its own files as one disc instead (no hairball of dots). Links of a
// file that is not drawn go to its stand-in, so the learned structure stays visible when zoomed out.
import { HubState, type LevelOfDetail } from '../layout/lod';
import type { MapLayout } from '../layout/radial';

/** What one level of detail draws. */
export interface Visibility {
  /** Hubs drawn as a square, in hub order. */
  readonly openHubs: Uint32Array;
  /** Hubs drawn as one node for their whole sub-tree, in hub order. */
  readonly collapsedHubs: Uint32Array;
  /** Whether open hubs draw their files one by one (true) or as one disc each (false). */
  readonly areFilesShown: boolean;
  /** Files drawn as circles, in id order; empty while files are not shown. */
  readonly visibleFiles: Uint32Array;
  /** For each file: -1 when drawn itself, else the hub drawn in its place. */
  readonly fileStandIn: Int32Array;
  /** For each hub: -1 when open, itself when collapsed, its collapsed ancestor when hidden. */
  readonly hubStandIn: Int32Array;
}

function hubStandIns(layout: MapLayout, lod: LevelOfDetail): Int32Array {
  const standIn = new Int32Array(layout.hubParent.length);
  // Pre-order lists a parent before its children, so a hidden hub's parent is already settled.
  for (let h = 0; h < standIn.length; h += 1) {
    const state = lod.state[h];
    if (state === HubState.Open) standIn[h] = -1;
    else if (state === HubState.Collapsed) standIn[h] = h;
    else standIn[h] = standIn[layout.hubParent[h] ?? 0] ?? -1;
  }
  return standIn;
}

function hubsIn(lod: LevelOfDetail, wanted: number): Uint32Array {
  let count = 0;
  for (const state of lod.state) if (state === wanted) count += 1;
  const hubs = new Uint32Array(count);
  let next = 0;
  for (let h = 0; h < lod.state.length; h += 1) {
    if (lod.state[h] === wanted) {
      hubs[next] = h;
      next += 1;
    }
  }
  return hubs;
}

/** The files whose stand-in is -1, in id order. */
function shownFiles(fileStandIn: Int32Array): Uint32Array {
  let count = 0;
  for (const standIn of fileStandIn) if (standIn < 0) count += 1;
  const files = new Uint32Array(count);
  let next = 0;
  for (let f = 0; f < fileStandIn.length; f += 1) {
    if ((fileStandIn[f] ?? 0) < 0) {
      files[next] = f;
      next += 1;
    }
  }
  return files;
}

/**
 * Computes what `lod` draws for `layout`. `areFilesShown` says whether the rings are far enough
 * apart on screen for single files; when false, a file of an open hub stands in as its hub.
 */
export function computeVisibility(
  layout: MapLayout,
  lod: LevelOfDetail,
  areFilesShown: boolean,
): Visibility {
  const hubStandIn = hubStandIns(layout, lod);
  const fileStandIn = new Int32Array(layout.fileHub.length);
  for (let f = 0; f < fileStandIn.length; f += 1) {
    const hub = layout.fileHub[f] ?? 0;
    const standIn = hubStandIn[hub] ?? -1;
    fileStandIn[f] = standIn >= 0 ? standIn : areFilesShown ? -1 : hub;
  }
  return {
    openHubs: hubsIn(lod, HubState.Open),
    collapsedHubs: hubsIn(lod, HubState.Collapsed),
    areFilesShown,
    visibleFiles: shownFiles(fileStandIn),
    fileStandIn,
    hubStandIn,
  };
}
