// Tests for the label collision rule, with boxes laid out by hand on a 200 x 100 screen.
import { describe, expect, it } from 'vitest';
import { placeLabels, type LabelCandidate, type PlacementRules, type ScreenBox } from './labels';

const AREA: ScreenBox = { left: 0, top: 0, width: 200, height: 100 };

function rules(change: Partial<PlacementRules> = {}): PlacementRules {
  return { area: AREA, obstacles: [], paddingPx: 2, maxCount: 100, ...change };
}

/** A candidate with one spot: a 40 x 10 box at (left, top). */
function at(
  name: string,
  left: number,
  top: number,
  priority: number,
): LabelCandidate & { name: string } {
  return { name, priority, spots: [{ left, top, width: 40, height: 10 }] };
}

function names(placed: readonly { candidate: { name: string } }[]): string[] {
  return placed.map(({ candidate }) => candidate.name);
}

describe('placeLabels', () => {
  it('keeps two labels whose padded boxes do not touch', () => {
    // a spans x 10..50, b spans x 55..95: padded by 2 they reach 52 and 53, a gap of 1.
    expect(names(placeLabels([at('a', 10, 10, 1), at('b', 55, 10, 1)], rules()))).toEqual([
      'a',
      'b',
    ]);
  });

  it('drops the lower priority of two labels within the padding of each other', () => {
    // b starts at 52: padded, a reaches 52 and b starts at 50, so they overlap by 2.
    const placed = placeLabels([at('a', 10, 10, 1), at('b', 52, 10, 5)], rules());
    expect(names(placed)).toEqual(['b']);
  });

  it('breaks a tie in priority by candidate order, the same on every run', () => {
    const candidates = [at('first', 10, 10, 3), at('second', 20, 12, 3)];
    expect(names(placeLabels(candidates, rules()))).toEqual(['first']);
    expect(names(placeLabels(candidates, rules()))).toEqual(['first']);
  });

  it('takes the next spot when the first is taken', () => {
    const blocker = at('blocker', 10, 10, 9);
    const mover = {
      name: 'mover',
      priority: 1,
      spots: [
        { left: 20, top: 12, width: 40, height: 10 },
        { left: 20, top: 60, width: 40, height: 10 },
      ],
    };
    const placed = placeLabels([blocker, mover], rules());
    expect(placed.map(({ candidate, spot }) => `${candidate.name}:${spot}`)).toEqual([
      'blocker:0',
      'mover:1',
    ]);
  });

  it('refuses a spot that leaves the clear area, even partly', () => {
    const area = { left: 0, top: 30, width: 200, height: 70 };
    // The first box starts at y 25, above the area's top at 30; the second fits.
    const placed = placeLabels([at('cut', 10, 25, 1), at('inside', 100, 40, 1)], rules({ area }));
    expect(names(placed)).toEqual(['inside']);
  });

  it('never covers an obstacle', () => {
    const obstacles = [{ left: 30, top: 10, width: 6, height: 6 }];
    const placed = placeLabels(
      [at('over', 10, 10, 1), at('clear', 100, 10, 1)],
      rules({ obstacles }),
    );
    expect(names(placed)).toEqual(['clear']);
  });

  it('places at most maxCount labels, highest priority first', () => {
    const candidates = [at('low', 10, 10, 1), at('high', 60, 10, 3), at('mid', 110, 10, 2)];
    expect(names(placeLabels(candidates, rules({ maxCount: 2 })))).toEqual(['high', 'mid']);
  });

  it('finds overlaps across grid cells (boxes far apart in the list, near on screen)', () => {
    // 24 labels, 4 a row 45 apart and 15 apart in rows: padded, none touches another.
    const many = Array.from({ length: 24 }, (_, i) => at(`f${i}`, 0, 0, 0)).map((c, i) => ({
      ...c,
      spots: [{ left: (i % 4) * 45, top: Math.floor(i / 4) * 15, width: 38, height: 10 }],
    }));
    // A label across the grid cell edge at x 64, over f1 (x 45..83) and f2 (x 90..128), placed first.
    const straddler = at('straddler', 60, 0, 10);
    const placed = names(placeLabels([...many, straddler], rules()));
    expect(placed[0]).toBe('straddler');
    expect(placed.length).toBe(23);
    expect(placed).not.toContain('f1');
    expect(placed).not.toContain('f2');
  });
});
