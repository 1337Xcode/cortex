// Tests for each view's encoder on the seven-file tree, with every tone, count and colour worked
// out by hand. Colours are the channels Chromium paints each token in (see e2e/colors.spec.ts); a
// tinted fill is the ground moved 0.3 of the way to the tone, channel by channel, worked below.
//
// The files, ids 0 to 6, and their hubs (0 root, 1 a, 2 b, 3 b/deep, 4 c):
//   0 README.md       tokens 0       reads 0  touched 30 h ago   links 1
//   1 a/one.rs        tokens 900     reads 3  touched 1 h ago    links 2
//   2 a/two.rs        tokens 1,800   reads 0  never touched      links 1
//   3 b/deep/leaf.rs  tokens 2,600   reads 1  never touched      links 0
//   4 b/mid.rs        tokens 6,200   reads 2  touched 23 h ago   links 4
//   5 b/zed.rs        tokens 29,910  reads 5  never touched      links 1
//   6 c/only.rs       tokens 0       reads 0  never touched      links 1
// Links: 0-4, 1-2, 1-4, 4-5, 4-6 (5 links; file 4 has four of them).
import { describe, expect, it } from 'vitest';
import type { GraphFile, GraphLink } from '../api/graph';
import { levelOfDetail } from '../layout/lod';
import { encodeFiles } from '../map/fileEncoding';
import type { FrameStyle } from '../map/frameStyle';
import { encodeFileDiscs } from '../map/hubEncoding';
import { buildScene } from '../map/scene';
import { sevenFileSnapshot, testFile, testLink } from '../map/testScene';
import { computeVisibility } from '../map/visibility';
import { DARK_COLORS, LIGHT_COLORS, type ThemeColors } from '../theme/tokens';
import { couplingStep, couplingView } from './coupling';
import { defaultView } from './default';
import { fogView } from './fog';
import { heatStep, heatView } from './heat';
import { folderTonesOf, toneNearest } from './folderTones';
import { paintView } from './paint';
import { recentView } from './recent';
import { VIEWS } from './registry';
import type { MapViewDefinition } from './view';

const NOW_MS = 1_762_900_000_000;
const HOUR_MS = 3_600_000;

const CHANGES = new Map<number, Partial<GraphFile>>([
  [0, { touched_ms: NOW_MS - 30 * HOUR_MS }],
  [1, { tokens: 900, reads: 3, touched_ms: NOW_MS - HOUR_MS }],
  [2, { tokens: 1800 }],
  [3, { tokens: 2600, reads: 1 }],
  [4, { tokens: 6200, reads: 2, touched_ms: NOW_MS - 23 * HOUR_MS }],
  [5, { tokens: 29910, reads: 5 }],
]);

const LINKS: GraphLink[] = [
  testLink(0, 4, 'co_change', 0.5),
  testLink(1, 2, 'co_change', 0.5),
  testLink(1, 4, 'used_together', 0.5),
  testLink(4, 5, 'work_order', 0.5),
  testLink(4, 6, 'work_order', 0.5),
];

const scene = buildScene(sevenFileSnapshot(LINKS, CHANGES));
const visibility = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), true);

/** The view's tones as "key name count", in the legend's order. */
function tonesOf(view: MapViewDefinition): string[] {
  return view.encode(scene, NOW_MS).tones.map((tone) => `${tone.key} ${tone.name} ${tone.count}`);
}

/** Each file's tone key, by id. */
function toneKeys(view: MapViewDefinition): string[] {
  const encoding = view.encode(scene, NOW_MS);
  return Array.from(encoding.toneOf, (index) => encoding.tones[index]?.key ?? '');
}

/** File `id`'s outline and fill channels in `view`, as the file encoder writes them. */
function channelsOf(view: MapViewDefinition, id: number, colors: ThemeColors = DARK_COLORS) {
  const style: FrameStyle = {
    colors,
    nowMs: NOW_MS,
    neighbourhood: null,
    selected: null,
    minStrength: 0,
    view: paintView(view, scene, NOW_MS),
  };
  const files = encodeFiles(scene, visibility, style);
  const row = Array.from(files.ids).indexOf(id);
  return {
    line: Array.from(files.lineColors.subarray(row * 4, row * 4 + 3)),
    fill: Array.from(files.fillColors.subarray(row * 4, row * 4 + 3)),
    widthTenths: Math.round((files.lineWidths[row] ?? 0) * 10),
  };
}

/** Whether every channel is within one step of 255 of the expected one (our conversion and Chromium's round apart). */
function near(actual: readonly number[], expected: readonly number[]): boolean {
  return expected.every((value, i) => Math.abs((actual[i] ?? -9) - value) <= 1);
}

describe('the registry', () => {
  it('lists five views in key order, the default first, each name once', () => {
    expect(VIEWS.map((view) => view.name)).toEqual([
      'default',
      'fog',
      'heat',
      'coupling',
      'recent',
    ]);
    expect(VIEWS.map((view) => view.title)).toEqual([
      'Default',
      'Fog',
      'Heat',
      'Coupling',
      'Recent',
    ]);
  });
});

describe('the default view', () => {
  it('sorts files by their mark and keeps every mark in the legend', () => {
    // Recent: a/one (1 h) and b/mid (23 h); never read: README, a/two, c/only; plain: leaf, zed.
    expect(tonesOf(defaultView)).toEqual([
      'recent Read or edited in the last 24 hours 2',
      'suspect Has a suspect fact 0',
      'unread Never read by an agent 3',
      'plain Other files 2',
    ]);
  });

  it('fills every file with the ground, as the map always has', () => {
    // Dark ground #0c1015; a recent file's outline green #82c984.
    const recent = channelsOf(defaultView, 1);
    expect(near(recent.line, [130, 201, 132])).toBe(true);
    expect(near(recent.fill, [12, 16, 21])).toBe(true);
  });
});

describe('the Fog view', () => {
  it('lights the read files and leaves the rest in fog, with the sentence that says so', () => {
    const encoding = fogView.encode(scene, NOW_MS);
    expect(tonesOf(fogView)).toEqual(['fog-lit Read by an agent 4', 'fog-dim Never read 3']);
    expect(toneKeys(fogView)).toEqual([
      'fog-dim',
      'fog-lit',
      'fog-dim',
      'fog-lit',
      'fog-lit',
      'fog-lit',
      'fog-dim',
    ]);
    expect(encoding.scale).toBe('Agents have read 4 files of 7; the other 3 are in fog.');
  });

  it('draws a lit file bright and filled, a fogged one dim, thin and unfilled', () => {
    // Lit #eaeff7; fill 12 + 0.3 x (234 - 12) = 78.6, 16 + 0.3 x 223 = 82.9, 21 + 0.3 x 226 = 88.8.
    const lit = channelsOf(fogView, 1);
    expect(near(lit.line, [234, 239, 247])).toBe(true);
    expect(near(lit.fill, [79, 83, 89])).toBe(true);
    expect(lit.widthTenths).toBe(11);
    // Never read #65696f, the thin 0.6 px outline, the ground for a fill.
    const fog = channelsOf(fogView, 0);
    expect(near(fog.line, [101, 105, 112])).toBe(true);
    expect(near(fog.fill, [12, 16, 21])).toBe(true);
    expect(fog.widthTenths).toBe(6);
  });

  it('says the whole map is in fog when no file was read', () => {
    const cold = buildScene(sevenFileSnapshot());
    const encoding = fogView.encode(cold, NOW_MS);
    expect(encoding.tones.map((tone) => tone.count)).toEqual([0, 7]);
    expect(encoding.scale).toBe('No agent has read a file here yet, so the whole map is in fog.');
  });
});

describe('the Heat view', () => {
  it('steps tokens on a log scale from the fewest to the most', () => {
    // ln 900 = 6.8024, ln 29,910 = 10.3061, range 3.5037. 1,800: (7.4955 - 6.8024) / 3.5037 =
    // 0.198, x 4 = 0.79, step 1; 2,600: 0.303 x 4 = 1.21, step 2; 6,200: 0.551 x 4 = 2.20, step 3.
    expect([900, 1800, 2600, 6200, 29910].map((v) => heatStep(v, 900, 29910))).toEqual([
      1, 1, 2, 3, 4,
    ]);
    expect([heatStep(0, 900, 29910), heatStep(5, 5, 5)]).toEqual([0, 4]);
  });

  it('names each step by the tokens its files have, strongest first', () => {
    expect(tonesOf(heatView)).toEqual([
      'heat-4 29,910 tokens 1',
      'heat-3 6,200 tokens 1',
      'heat-2 2,600 tokens 1',
      'heat-1 900 to 1,800 tokens 2',
      'heat-none No tokens attributed 2',
    ]);
    expect(heatView.encode(scene, NOW_MS).scale).toBe(
      'Tokens attributed (input, output, cache writes and cache reads), on a log scale from 900 to 29,910; 2 files with none.',
    );
  });

  it('draws the strongest step in the accent and the weakest darker, each filled 0.3 of the way', () => {
    // Step 4 #b3a7fb: fill 12 + 0.3 x 167 = 62.1, 16 + 0.3 x 151 = 61.3, 21 + 0.3 x 230 = 90.
    const hot = channelsOf(heatView, 5);
    expect(near(hot.line, [179, 167, 251])).toBe(true);
    expect(near(hot.fill, [62, 61, 90])).toBe(true);
    // Step 1 #6c5eac: fill 12 + 0.3 x 96 = 40.8, 16 + 0.3 x 78 = 39.4, 21 + 0.3 x 151 = 66.3.
    const cool = channelsOf(heatView, 1);
    expect(near(cool.line, [108, 94, 172])).toBe(true);
    expect(near(cool.fill, [41, 39, 66])).toBe(true);
  });

  it('says when no tokens are attributed at all', () => {
    const cold = heatView.encode(buildScene(sevenFileSnapshot()), NOW_MS);
    expect(cold.tones.map((tone) => `${tone.key} ${tone.count}`)).toEqual(['heat-none 7']);
    expect(cold.scale).toBe('No tokens have been attributed to any file yet.');
  });
});

describe('the Coupling view', () => {
  it('steps link counts linearly up to the most', () => {
    // The most is 4: 1 link ceil(1) = 1, 2 links ceil(2) = 2, 3 links ceil(3) = 3, 4 links 4.
    expect([0, 1, 2, 3, 4].map((links) => couplingStep(links, 4))).toEqual([0, 1, 2, 3, 4]);
    // With 5 the most, 4 links is ceil(3.2) = 4.
    expect(couplingStep(4, 5)).toBe(4);
  });

  it('names each step by its link counts and leaves unlinked files dim', () => {
    expect(tonesOf(couplingView)).toEqual([
      'coupling-4 4 links 1',
      'coupling-2 2 links 1',
      'coupling-1 1 link 4',
      'coupling-none No learned links 1',
    ]);
    expect(couplingView.encode(scene, NOW_MS).scale).toBe(
      "Links the map keeps per file (each file's 8 strongest of each kind, and those its neighbours keep to it), from 1 to 4; 1 file with none.",
    );
  });

  it('counts the links the map keeps, never the learned links it left out', () => {
    // The daemon learned a/one.rs-a/two.rs too, but kept only README.md-b/mid.rs: the graph holds
    // one link, so README.md and b/mid.rs have 1 each, the other five none, and the sentence
    // names the kept set.
    const kept = buildScene(sevenFileSnapshot([testLink(0, 4, 'co_change', 0.5)]));
    const encoding = couplingView.encode(kept, NOW_MS);
    expect(encoding.tones.map((tone) => `${tone.key} ${tone.count}`)).toEqual([
      'coupling-4 2',
      'coupling-none 5',
    ]);
    expect(encoding.scale).toMatch(/^Links the map keeps per file \(each file's 8 strongest/);
    expect(encoding.scale).not.toContain('Learned links');
  });

  it('writes its counts with thousands separators', () => {
    // A hub file linked to 1,234 others has 1,234 links; each of them has 1.
    const paths = Array.from({ length: 1235 }, (_, id) => `f${String(id).padStart(4, '0')}.rs`);
    const files = paths.map((path, id) => testFile(id, path));
    const links = Array.from({ length: 1234 }, (_, at) => testLink(0, at + 1, 'co_change', 0.5));
    const big = buildScene({ version: 1, files, links });
    expect(couplingView.encode(big, NOW_MS).scale).toBe(
      "Links the map keeps per file (each file's 8 strongest of each kind, and those its neighbours keep to it), from 1 to 1,234.",
    );
  });

  it('draws the most linked file in the light theme deep violet, filled 0.3 of the way', () => {
    // Light step 4 #483a7e on the ground #f5f7f9: 245 - 0.3 x 173 = 193.1, 247 - 0.3 x 189 =
    // 190.3, 249 - 0.3 x 123 = 212.1.
    const most = channelsOf(couplingView, 4, LIGHT_COLORS);
    expect(near(most.line, [72, 58, 126])).toBe(true);
    expect(near(most.fill, [193, 190, 212])).toBe(true);
  });
});

describe('the Recent view', () => {
  it('lights the files touched in the 24 hours before now', () => {
    // a/one 1 h and b/mid 23 h ago are in; README 30 h ago is out.
    expect(tonesOf(recentView)).toEqual([
      'recent-lit Read or edited in the last 24 hours 2',
      'recent-dim Not in the last 24 hours 5',
    ]);
    expect(recentView.encode(scene, NOW_MS).scale).toBe(
      '2 files of 7 read or edited by an agent in the 24 hours before the map last read its data.',
    );
  });

  it('writes the file count with thousands separators, as Fog does', () => {
    // 1,234 files, the first touched an hour ago and read once: Recent lights 1, Fog lights 1 and
    // leaves 1,233 in fog.
    const paths = Array.from({ length: 1234 }, (_, id) => `f${String(id).padStart(4, '0')}.rs`);
    const files = paths.map((path, id) =>
      testFile(id, path, id === 0 ? { touched_ms: NOW_MS - HOUR_MS, reads: 1 } : {}),
    );
    const big = buildScene({ version: 1, files, links: [] });
    expect(recentView.encode(big, NOW_MS).scale).toBe(
      '1 file of 1,234 read or edited by an agent in the 24 hours before the map last read its data.',
    );
    expect(fogView.encode(big, NOW_MS).scale).toBe(
      'Agents have read 1 file of 1,234; the other 1,233 are in fog.',
    );
  });

  it('draws a recent file green and filled 0.3 of the way', () => {
    // Green #82c984: 12 + 0.3 x 118 = 47.4, 16 + 0.3 x 185 = 71.5, 21 + 0.3 x 111 = 54.3.
    const lit = channelsOf(recentView, 4);
    expect(near(lit.line, [130, 201, 132])).toBe(true);
    expect(near(lit.fill, [47, 72, 54])).toBe(true);
  });
});

describe('folders', () => {
  /** Each folder's tone key in `view`, for its own files and for its sub-tree, hubs 0 to 4. */
  function folderKeys(view: MapViewDefinition): { own: string[]; tree: string[] } {
    const encoding = view.encode(scene, NOW_MS);
    const folders = folderTonesOf(scene, encoding);
    const keyOf = (index: number): string => encoding.tones[index]?.key ?? '-';
    return { own: Array.from(folders.own, keyOf), tree: Array.from(folders.tree, keyOf) };
  }

  it('in Fog and Recent take their strongest file: one read or recent file lights the folder', () => {
    // Own files: root README (fog), a one (lit) and two (fog), b mid and zed (lit), b/deep leaf
    // (lit), c only (fog). Recent: a/one and b/mid only.
    expect(folderKeys(fogView).own).toEqual([
      'fog-dim',
      'fog-lit',
      'fog-lit',
      'fog-lit',
      'fog-dim',
    ]);
    expect(folderKeys(recentView).tree).toEqual([
      'recent-lit',
      'recent-lit',
      'recent-lit',
      'recent-dim',
      'recent-dim',
    ]);
  });

  it('in Heat take their typical file: the mean step, rounded, of every file under them', () => {
    // Steps: README 0, one 1, two 1, leaf 2, mid 3, zed 4, only 0. Root: 11 / 7 = 1.57, step 2;
    // a: (1 + 1) / 2 = 1; b: (2 + 3 + 4) / 3 = 3; b/deep: 2; c: 0, none.
    expect(folderKeys(heatView).tree).toEqual([
      'heat-2',
      'heat-1',
      'heat-3',
      'heat-2',
      'heat-none',
    ]);
  });

  it('take the nearest tone a file shows when the rounded step has none, the lower on a tie', () => {
    // Coupling shows steps 4, 2, 1 and none: a mean of 3 has no tone of its own; 2 and 4 are as
    // near, and the lower wins.
    const encoding = couplingView.encode(scene, NOW_MS);
    const key = (rank: number): string => encoding.tones[toneNearest(encoding, rank)]?.key ?? '-';
    expect([key(3), key(4), key(0)]).toEqual(['coupling-2', 'coupling-4', 'coupling-none']);
  });

  it('drawn as discs are edged in their tone, and stay neutral in the default view', () => {
    // While rings are too close to draw files one by one, each open folder draws its own files as
    // one disc. Folder b (hub 2) holds b/mid (step 3) and b/zed (step 4): (3 + 4) / 2 = 3.5
    // rounds to step 4, #b3a7fb. In the default view every disc keeps the never-read grey #65696f.
    const far = computeVisibility(scene.layout, levelOfDetail(scene.layout, 1), false);
    const edgeOf = (view: MapViewDefinition): number[] => {
      const style: FrameStyle = {
        colors: DARK_COLORS,
        nowMs: NOW_MS,
        neighbourhood: null,
        selected: null,
        minStrength: 0,
        view: paintView(view, scene, NOW_MS),
      };
      const discs = encodeFileDiscs(scene, far, style, new Uint8Array(5));
      const row = Array.from(discs.ids).indexOf(2);
      return Array.from(discs.lineColors.subarray(row * 4, row * 4 + 3));
    };
    expect(near(edgeOf(heatView), [179, 167, 251])).toBe(true);
    expect(near(edgeOf(defaultView), [101, 105, 112])).toBe(true);
  });
});
