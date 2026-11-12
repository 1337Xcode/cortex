// The tone a folder takes in a view that tints, so a folder drawn whole still shows the view's
// answer. A view chooses its rule: the strongest tone among the folder's files, where one file is
// the answer (any file read lights the Fog; any file touched today lights Recent), or its files'
// typical tone, the mean of their ranks rounded, where a strongest file would light every folder
// of a large repository alike (Heat, Coupling). Computed once per view and snapshot, for the
// folder's own files (its disc while files are drawn as discs) and for its whole sub-tree (drawn
// whole).
import type { Scene } from '../map/scene';
import type { ViewEncoding } from './view';

/** Marks a folder with no file in it. */
export const NO_TONE = 255;

/** Each folder's tone, as an index into the view's tones, or `NO_TONE`. */
export interface FolderTones {
  /** From the folder's own files. */
  readonly own: Uint8Array;
  /** From every file under the folder. */
  readonly tree: Uint8Array;
}

/** Each folder's sum of its files' ranks and its file count, own files and sub-tree. */
interface RankSums {
  readonly ownSum: Float64Array;
  readonly ownCount: Uint32Array;
  readonly treeSum: Float64Array;
  readonly treeCount: Uint32Array;
  readonly ownMax: Int16Array;
  readonly treeMax: Int16Array;
}

/** Folds every file's rank into its folder, then every finished sub-tree into its parent. */
function rankSums(scene: Scene, view: ViewEncoding): RankSums {
  const { fileHub, hubParent } = scene.layout;
  const hubs = hubParent.length;
  const ownSum = new Float64Array(hubs);
  const ownCount = new Uint32Array(hubs);
  const ownMax = new Int16Array(hubs).fill(-1);
  for (let id = 0; id < fileHub.length; id += 1) {
    const hub = fileHub[id] ?? 0;
    const rank = view.tones[view.toneOf[id] ?? 0]?.rank ?? 0;
    ownSum[hub] = (ownSum[hub] ?? 0) + rank;
    ownCount[hub] = (ownCount[hub] ?? 0) + 1;
    ownMax[hub] = Math.max(ownMax[hub] ?? -1, rank);
  }
  const treeSum = ownSum.slice();
  const treeCount = ownCount.slice();
  const treeMax = ownMax.slice();
  // Hubs are in pre-order, so every child comes after its parent: walking backwards folds each
  // finished sub-tree into its parent.
  for (let hub = hubs - 1; hub > 0; hub -= 1) {
    const parent = hubParent[hub] ?? 0;
    treeSum[parent] = (treeSum[parent] ?? 0) + (treeSum[hub] ?? 0);
    treeCount[parent] = (treeCount[parent] ?? 0) + (treeCount[hub] ?? 0);
    treeMax[parent] = Math.max(treeMax[parent] ?? -1, treeMax[hub] ?? -1);
  }
  return { ownSum, ownCount, treeSum, treeCount, ownMax, treeMax };
}

/** The tone of `view` whose rank is nearest `rank`, the lower on a tie. */
export function toneNearest(view: ViewEncoding, rank: number): number {
  let best = NO_TONE;
  let bestDistance = Infinity;
  for (const [index, tone] of view.tones.entries()) {
    const distance = Math.abs(tone.rank - rank);
    const isLowerTie = distance === bestDistance && tone.rank < (view.tones[best]?.rank ?? 0);
    if (distance < bestDistance || isLowerTie) {
      best = index;
      bestDistance = distance;
    }
  }
  return best;
}

/** Each folder's tone in `view` by the view's folder rule, for its own files and its sub-tree. */
export function folderTonesOf(scene: Scene, view: ViewEncoding): FolderTones {
  const sums = rankSums(scene, view);
  const toneOf = (sum: number, count: number, max: number): number => {
    if (count === 0) return NO_TONE;
    return toneNearest(view, view.folderRule === 'strongest' ? max : Math.round(sum / count));
  };
  const hubs = scene.layout.hubParent.length;
  const own = new Uint8Array(hubs);
  const tree = new Uint8Array(hubs);
  for (let hub = 0; hub < hubs; hub += 1) {
    own[hub] = toneOf(sums.ownSum[hub] ?? 0, sums.ownCount[hub] ?? 0, sums.ownMax[hub] ?? -1);
    tree[hub] = toneOf(sums.treeSum[hub] ?? 0, sums.treeCount[hub] ?? 0, sums.treeMax[hub] ?? -1);
  }
  return { own, tree };
}
