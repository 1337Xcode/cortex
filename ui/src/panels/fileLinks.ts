// A file's strongest learned links, grouped by kind for the side panel: strongest first, ties by
// the other file's path, at most a few per kind. Read from the snapshot's adjacency, so it costs
// only the file's own links.
import { LINK_KINDS, type GraphLink, type LinkKind } from '../api/graph';
import type { Scene } from '../map/scene';
import { compareBytes } from '../text/byteOrder';

/** One link as the panel lists it: the other file, the weight and, for co-change, the commits. */
export interface PanelLink {
  readonly other: number;
  readonly path: string;
  readonly weight: number;
  readonly commits: number;
}

/** Up to `perKind` links of each kind for file `id`, strongest first. */
export function strongestLinks(
  scene: Scene,
  id: number,
  perKind: number,
): Readonly<Record<LinkKind, readonly PanelLink[]>> {
  const grouped: Record<LinkKind, PanelLink[]> = {
    co_change: [],
    used_together: [],
    work_order: [],
  };
  const { offsets, linkIds } = scene.adjacency;
  for (let slot = offsets[id] ?? 0; slot < (offsets[id + 1] ?? 0); slot += 1) {
    const link: GraphLink | undefined = scene.snapshot.links[linkIds[slot] ?? 0];
    if (link === undefined) continue;
    const other = link.from === id ? link.to : link.from;
    const path = scene.snapshot.files[other]?.path ?? '';
    grouped[link.kind].push({ other, path, weight: link.weight, commits: link.commits });
  }
  for (const kind of LINK_KINDS) {
    grouped[kind].sort((a, b) => b.weight - a.weight || compareBytes(a.path, b.path));
    grouped[kind] = grouped[kind].slice(0, perKind);
  }
  return grouped;
}
