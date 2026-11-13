// A file's strongest learned links, grouped by kind for the side panel: strongest first as the
// daemon ranks them (a co-change link by its strength times ln(1 + commits), another kind by its
// strength), then by the count its strength was made from, then by the other file's path; at most a
// few per kind, with how many there are. Read from the snapshot's adjacency, so it costs only the
// file's own links.
import { LINK_KINDS, type GraphLink, type LinkKind } from '../api/graph';
import type { Scene } from '../map/scene';
import { compareBytes } from '../text/byteOrder';
import { pairRank } from './overviewLists';

/** One link as the panel lists it: the other file, the weight and what it was made from. */
export interface PanelLink {
  readonly other: number;
  readonly path: string;
  readonly weight: number;
  /** For co-change, the commits that changed both files; 0 for the other kinds. */
  readonly commits: number;
  /** What the weight was made from: commits, tasks or work orders, by kind. */
  readonly count: number;
}

/** A file's links of each kind, the strongest first, and how many of each the map keeps. */
export interface FileLinks {
  readonly shown: Readonly<Record<LinkKind, readonly PanelLink[]>>;
  readonly totals: Readonly<Record<LinkKind, number>>;
}

/** How strong a link is for ranking, as the daemon ranks its kind. */
function rankOf(kind: LinkKind, link: PanelLink): number {
  return kind === 'co_change' ? pairRank(link) : link.weight;
}

/** Up to `perKind` links of each kind for file `id`, strongest first, and every kind's total. */
export function strongestLinks(scene: Scene, id: number, perKind: number): FileLinks {
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
    grouped[link.kind].push({
      other,
      path,
      weight: link.weight,
      commits: link.commits,
      count: link.count,
    });
  }
  const totals = { co_change: 0, used_together: 0, work_order: 0 };
  for (const kind of LINK_KINDS) {
    totals[kind] = grouped[kind].length;
    // INVARIANT: deterministic (ties by count, then by the other file's path)
    grouped[kind].sort(
      (a, b) =>
        rankOf(kind, b) - rankOf(kind, a) || b.count - a.count || compareBytes(a.path, b.path),
    );
    grouped[kind] = grouped[kind].slice(0, perKind);
  }
  return { shown: grouped, totals };
}
