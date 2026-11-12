// Label placement with collision avoidance. Each candidate offers a few spots around its node, in
// order of preference (below, above, right, left). Candidates are placed in priority order, each
// at its first spot whose box, grown by a padding, overlaps no label already placed and lies in
// the clear area; a candidate with no such spot is dropped. A uniform grid of screen cells keeps
// each check to the few labels nearby.

/** A box on screen: left and top edges, width and height, in pixels. */
export interface ScreenBox {
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/** A label that may be drawn: where it could sit and how much it matters. */
export interface LabelCandidate {
  /** Where the label's box could go, most preferred first. */
  readonly spots: readonly ScreenBox[];
  /** Higher places first; equal priorities keep the candidates' order. */
  readonly priority: number;
}

/** A placed candidate and the index of the spot it took. */
export interface PlacedLabel<T extends LabelCandidate> {
  readonly candidate: T;
  readonly spot: number;
}

/** The screen's size in pixels. */
export interface ScreenSize {
  readonly width: number;
  readonly height: number;
}

/** The side of one grid cell in pixels; about one short label wide. */
const CELL_PX = 64;

interface Edges {
  readonly left: number;
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
}

function edgesOf(box: ScreenBox, padding: number): Edges {
  return {
    left: box.left - padding,
    top: box.top - padding,
    right: box.left + box.width + padding,
    bottom: box.top + box.height + padding,
  };
}

function overlaps(a: Edges, b: Edges): boolean {
  return a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
}

function cellRange(edges: Edges): [number, number, number, number] {
  return [
    Math.floor(edges.left / CELL_PX),
    Math.floor(edges.top / CELL_PX),
    Math.floor(edges.right / CELL_PX),
    Math.floor(edges.bottom / CELL_PX),
  ];
}

/** Placed boxes bucketed by grid cell. */
class BoxGrid {
  private readonly cells = new Map<string, Edges[]>();

  hits(edges: Edges): boolean {
    const [x0, y0, x1, y1] = cellRange(edges);
    for (let x = x0; x <= x1; x += 1) {
      for (let y = y0; y <= y1; y += 1) {
        const placed = this.cells.get(`${x},${y}`);
        if (placed?.some((other) => overlaps(other, edges)) === true) return true;
      }
    }
    return false;
  }

  add(edges: Edges): void {
    const [x0, y0, x1, y1] = cellRange(edges);
    for (let x = x0; x <= x1; x += 1) {
      for (let y = y0; y <= y1; y += 1) {
        const key = `${x},${y}`;
        const list = this.cells.get(key);
        if (list === undefined) this.cells.set(key, [edges]);
        else list.push(edges);
      }
    }
  }
}

function isInside(box: ScreenBox, area: ScreenBox): boolean {
  return (
    box.left >= area.left &&
    box.top >= area.top &&
    box.left + box.width <= area.left + area.width &&
    box.top + box.height <= area.top + area.height
  );
}

/** The rules every label must keep. */
export interface PlacementRules {
  /** The clear part of the screen; a label lies wholly inside it. */
  readonly area: ScreenBox;
  /** Boxes no label may cover: the drawn nodes. */
  readonly obstacles: readonly ScreenBox[];
  /** Clear space kept around every label, in pixels. */
  readonly paddingPx: number;
  readonly maxCount: number;
}

/**
 * Chooses the labels to draw: in priority order, each candidate at its first spot that lies in
 * the area and whose box, grown by the padding on every side, overlaps no obstacle and no label
 * already placed; at most `maxCount`. Returns the placed candidates in placement order.
 */
export function placeLabels<T extends LabelCandidate>(
  candidates: readonly T[],
  rules: PlacementRules,
): PlacedLabel<T>[] {
  const { area, obstacles, paddingPx, maxCount } = rules;
  // INVARIANT: deterministic (ties keep the candidates' order, so the same input places the same labels)
  const order = candidates
    .map((candidate, index) => ({ candidate, index }))
    .sort((a, b) => b.candidate.priority - a.candidate.priority || a.index - b.index);
  const grid = new BoxGrid();
  for (const obstacle of obstacles) grid.add(edgesOf(obstacle, 0));
  const placed: PlacedLabel<T>[] = [];
  for (const { candidate } of order) {
    if (placed.length >= maxCount) break;
    const spot = candidate.spots.findIndex(
      (box) => isInside(box, area) && !grid.hits(edgesOf(box, paddingPx)),
    );
    const box = candidate.spots[spot];
    if (box === undefined) continue;
    grid.add(edgesOf(box, paddingPx));
    placed.push({ candidate, spot });
  }
  return placed;
}
