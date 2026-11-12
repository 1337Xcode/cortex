// The neighbourhood of the hovered or selected file: the file itself and every file one learned
// link away (counting only links at or above the strength threshold). Everything outside it is
// drawn dimmed while a focus is set.
import type { Scene } from './scene';

/** The focus file and its neighbours. */
export interface Neighbourhood {
  readonly focus: number;
  /** 1 for the focus file and each neighbour, 0 for the rest; indexed by file id. */
  readonly isMember: Uint8Array;
  /** The neighbours' ids in ascending order, without the focus file. */
  readonly neighbours: readonly number[];
}

/** Computes the neighbourhood of `focus`, or null when there is no focus. */
export function neighbourhoodOf(
  scene: Scene,
  focus: number | null,
  minStrength: number,
): Neighbourhood | null {
  if (focus === null || focus < 0 || focus >= scene.snapshot.files.length) return null;
  const { offsets, linkIds } = scene.adjacency;
  const isMember = new Uint8Array(scene.snapshot.files.length);
  isMember[focus] = 1;
  const neighbours: number[] = [];
  for (let slot = offsets[focus] ?? 0; slot < (offsets[focus + 1] ?? 0); slot += 1) {
    const link = scene.snapshot.links[linkIds[slot] ?? 0];
    if (link === undefined || link.weight < minStrength) continue;
    const other = link.from === focus ? link.to : link.from;
    if (isMember[other] === 1) continue;
    isMember[other] = 1;
    neighbours.push(other);
  }
  neighbours.sort((a, b) => a - b);
  return { focus, isMember, neighbours };
}
