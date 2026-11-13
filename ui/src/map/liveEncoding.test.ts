// Tests for the live markers and the fact badges on the seven-file tree, values worked out by hand:
// a diamond's size around its file, nested diamonds for sessions on one spot, a trail's steps and
// their fading, a file the snapshot does not hold, markers at a folder when files are not drawn,
// and the badges' place, colour and the zoom they appear at.
import { describe, expect, it } from 'vitest';
import type { LiveSession } from '../api/live';
import { levelOfDetail } from '../layout/lod';
import { DARK_COLORS } from '../theme/tokens';
import { areBadgesShown, badgeSpot, encodeBadges } from './badges';
import type { FrameStyle } from './frameStyle';
import { encodeLive, markerRadiusPx, trailOpacity } from './liveEncoding';
import { buildScene } from './scene';
import { defaultPaint, SEVEN_PATHS, sevenFileSnapshot } from './testScene';
import { computeVisibility } from './visibility';

const scene = buildScene(
  sevenFileSnapshot(
    [],
    new Map([
      [4, { facts: 2, suspect: 1 }],
      [5, { facts: 1 }],
    ]),
  ),
);
const shown = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), true);

/**
 * A session on `files`, newest first: each an id, named by its path on the seven-file tree (or a
 * path that tree does not hold, past its seven), or an id and the path the frame gives with it.
 */
function session(
  id: string,
  agent: string,
  files: readonly (number | readonly [number, string])[],
): LiveSession {
  return {
    id,
    agent,
    title: agent,
    files: files.map((entry, index) => {
      const [file, path] =
        typeof entry === 'number' ? [entry, SEVEN_PATHS[entry] ?? `new/f${entry}.rs`] : entry;
      return { file, path, mode: 'read', last_ms: 1000 - index };
    }),
  };
}

describe('markerRadiusPx', () => {
  it('clears the file circle with the diamond, never smaller than 7 px, 4 px more per stacked session', () => {
    // A 1.9-unit circle at 4 px a unit is 7.6 px; a diamond around it needs 7.6 x 1.4142 = 10.748,
    // plus the 3 px gap: 13.748.
    expect(markerRadiusPx(1.9, 4, 0)).toBeCloseTo(13.748, 3);
    expect(markerRadiusPx(1.9, 4, 2)).toBeCloseTo(21.748, 3);
    // Far out the circle is 0.19 px: the 7 px floor holds.
    expect(markerRadiusPx(1.9, 0.1, 0)).toBe(7);
  });
});

describe('trailOpacity', () => {
  it('fades evenly from 0.5 at the newest step to 0.1 at the ninth', () => {
    // Nine steps between 0.5 and 0.1: each 0.4 / 9 = 0.0444 fainter.
    expect([0, 1, 9, 12].map((index) => Number(trailOpacity(index).toFixed(4)))).toEqual([
      0.5, 0.4556, 0.1, 0.1,
    ]);
  });
});

describe('encodeLive', () => {
  const colors = { colors: DARK_COLORS, scale: 4 };

  it('marks each session on its newest file, in its agent colour, and nests sessions on one spot', () => {
    const sessions = [
      session('a', 'claude', [4, 1]),
      session('b', 'codex', [4]),
      session('c', 'aider', [6]),
    ];
    const live = encodeLive(scene, shown, sessions, colors);
    // b/mid.rs (4) is at its place in the layout; c/only.rs (6) too.
    expect(Array.from(live.markers.positions)).toEqual([
      scene.layout.fileX[4],
      scene.layout.fileY[4],
      scene.layout.fileX[4],
      scene.layout.fileY[4],
      scene.layout.fileX[6],
      scene.layout.fileY[6],
    ]);
    // 100 lines: radius 1.9 units, 13.748 px; the second session on b/mid.rs 4 px larger.
    expect(Array.from(live.markers.radii).map((r) => Number(r.toFixed(2)))).toEqual([
      13.75, 17.75, 13.75,
    ]);
    // Claude rose #eb94c6 edge and a fill at 0.16 (41 of 255); Codex cyan #35cbd2; Aider the
    // shared neutral #b6bbc2 (as Chromium paints their tokens, within one step).
    const edge = (row: number): number[] =>
      Array.from(live.markers.lineColors.subarray(row * 4, row * 4 + 4));
    const near = (a: number[], b: number[]): boolean =>
      b.every((v, i) => Math.abs((a[i] ?? -9) - v) <= 1);
    expect(near(edge(0), [235, 148, 198, 255])).toBe(true);
    expect(live.markers.fillColors[3]).toBe(41);
    expect(near(edge(1), [53, 203, 210, 255])).toBe(true);
    expect(near(edge(2), [182, 187, 194, 255])).toBe(true);
  });

  it('draws the trail from the newest file back, each step fainter, with a dot on each earlier file', () => {
    const live = encodeLive(scene, shown, [session('a', 'claude', [4, 1, 6])], colors);
    // Steps 4 to 1 and 1 to 6, at 0.5 (128 of 255) and 0.4556 (116 of 255); dots on 1 and 6 at
    // 0.4556 and 0.4111 (105 of 255).
    expect(Array.from(live.trails.sources)).toEqual([
      scene.layout.fileX[4],
      scene.layout.fileY[4],
      scene.layout.fileX[1],
      scene.layout.fileY[1],
    ]);
    expect([live.trails.colors[3], live.trails.colors[7]]).toEqual([128, 116]);
    expect([live.stops.fillColors[3], live.stops.fillColors[7]]).toEqual([116, 105]);
  });

  it('leaves out files the snapshot does not hold, and a session with none at all', () => {
    const live = encodeLive(
      scene,
      shown,
      [session('a', 'claude', [99, 4]), session('b', 'codex', [42])],
      colors,
    );
    expect(live.sessions.map((entry) => entry.id)).toEqual(['a']);
    expect(live.files).toEqual([[4]]);
    expect(live.trails.ids).toHaveLength(0);
  });

  it('marks a file where the snapshot holds its path, never at the id the frame gave', () => {
    // The daemon's ids moved on: its 4 is c/only.rs (6 here) and its 5 a file this snapshot does
    // not hold yet, while b/zed.rs sits at 5 here. The marker goes on c/only.rs, and the file not
    // held is left out rather than marked on b/zed.rs.
    const live = encodeLive(
      scene,
      shown,
      [
        session('a', 'claude', [
          [4, 'c/only.rs'],
          [5, 'b/new.rs'],
        ]),
      ],
      colors,
    );
    expect(live.files).toEqual([[6]]);
    expect(Array.from(live.markers.positions)).toEqual([
      scene.layout.fileX[6],
      scene.layout.fileY[6],
    ]);
    expect(live.stops.ids).toHaveLength(0);
  });

  it('marks a file at its folder while files are drawn as discs, and skips a trail step inside it', () => {
    // b/mid.rs (4) and b/zed.rs (5) are both in folder b (hub 2).
    const discs = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), false);
    const live = encodeLive(scene, discs, [session('a', 'claude', [4, 5])], {
      ...colors,
      scale: 0.1,
    });
    expect(Array.from(live.markers.positions)).toEqual([
      scene.layout.hubX[2],
      scene.layout.hubY[2],
    ]);
    expect(live.markers.radii[0]).toBe(7);
    expect(live.trails.ids).toHaveLength(0);
    expect(live.stops.ids).toHaveLength(1);
  });
});

describe('fact badges', () => {
  const style: FrameStyle = {
    colors: DARK_COLORS,
    nowMs: 0,
    neighbourhood: null,
    selected: null,
    minStrength: 0,
    view: defaultPaint(scene),
  };

  it('appear only with file names: files drawn one by one at 2 px a unit or more', () => {
    expect(areBadgesShown(shown, 2)).toBe(true);
    expect(areBadgesShown(shown, 1.9)).toBe(false);
    const discs = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), false);
    expect(areBadgesShown(discs, 8)).toBe(false);
  });

  it('sit on the rim up and to the right, amber with a suspect fact and grey without', () => {
    // A radius of 2 at (10, 0): 2 x 0.7071 = 1.4142 right and up.
    const [x, y] = badgeSpot(10, 0, 2);
    expect([x.toFixed(4), y.toFixed(4)]).toEqual(['11.4142', '-1.4142']);
    const badges = encodeBadges(scene, shown, style, true);
    expect(Array.from(badges.ids)).toEqual([4, 5]);
    // Suspect amber #e1a956 on b/mid.rs; the badge grey #b6bbc2 on b/zed.rs; 3 px each.
    const near = (a: number[], b: number[]): boolean =>
      b.every((v, i) => Math.abs((a[i] ?? -9) - v) <= 1);
    expect(near(Array.from(badges.fillColors.subarray(0, 3)), [225, 169, 86])).toBe(true);
    expect(near(Array.from(badges.fillColors.subarray(4, 7)), [182, 187, 194])).toBe(true);
    expect(Array.from(badges.radii)).toEqual([3, 3]);
    expect(encodeBadges(scene, shown, style, false).ids).toHaveLength(0);
  });
});
