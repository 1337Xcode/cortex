// What a view is: a way to colour the same graph so it answers one question. A view sorts every
// file into one of a few tones (each a colour role, drawn dim or not, with the legend line that
// names it and how many files it holds) and says in one sentence what its scale means. Shape keeps
// its meaning in every view (square folder, circle file, second ring, filled centre); only the
// outline's colour and weight change. The views themselves are one file each, listed in
// `registry.ts`.
import type { Scene } from '../map/scene';
import type { Oklch } from '../theme/oklch';
import type { ThemeColors } from '../theme/tokens';

/** The colour a tone draws in, by its role in the theme. */
export type ToneRole =
  'file' | 'unread' | 'recent' | 'suspect' | 'lit' | 'scale1' | 'scale2' | 'scale3' | 'scale4';

/** How a folder drawn whole takes a tone from its files; see `ViewEncoding.folderRule`. */
export type FolderRule = 'strongest' | 'typical';

/** The second channel the legend's sample shows for a tone, as the canvas draws it. */
export type ToneMark = 'ring' | 'dot' | null;

/** One tone of a view: the files that share an outline. */
export interface Tone {
  /** Names the tone for the checks (`legend-<key>`), unique among all views. */
  readonly key: string;
  readonly role: ToneRole;
  /** Drawn with the thinner outline: the view puts these files in the background. */
  readonly isDim: boolean;
  /** How strongly the tone speaks; a folder drawn whole takes its strongest file's tone. */
  readonly rank: number;
  /** The legend's name for it, with its numbers where it has some ("2,600 to 4,100 tokens"). */
  readonly name: string;
  /** One line on how it is drawn, or empty when its place on the scale says it. */
  readonly meaning: string;
  readonly mark: ToneMark;
  /** How many files of the repository show it. */
  readonly count: number;
}

/** One view's colouring of one snapshot. */
export interface ViewEncoding {
  /** For each file id, the index of its tone in `tones`. */
  readonly toneOf: Uint8Array;
  /** The view's tones in the legend's order, strongest first. */
  readonly tones: readonly Tone[];
  /**
   * Whether the view tints: a file in a tone that is not dim is filled with a share of its tone, and
   * a folder drawn whole takes its strongest file's tone. The default view keeps fills and folders
   * neutral, as its tones are marks.
   */
  readonly isTinted: boolean;
  /**
   * Which tone a folder drawn whole takes in a view that tints: its strongest file's, where one
   * file is the answer, or its typical file's (the mean rank, rounded), where a strongest file
   * would light every folder alike.
   */
  readonly folderRule: FolderRule;
  /** What the scale means, in one sentence with its numbers. */
  readonly scale: string;
}

/** A view: its name, its title on the switch, the question it answers, and its encoder. */
export interface MapViewDefinition<N extends string = string> {
  readonly name: N;
  /** The word on the switch and at the head of the legend. */
  readonly title: string;
  /** The question the view answers, said by the switch's button. */
  readonly question: string;
  /** Colours `scene`; "the last 24 hours" ends at `nowMs`. */
  readonly encode: (scene: Scene, nowMs: number) => ViewEncoding;
}

/** Declares a view, keeping its name's literal type so the registry knows every name. */
export function defineView<const N extends string>(
  view: MapViewDefinition<N>,
): MapViewDefinition<N> {
  return view;
}

/** The colour of a tone's role in `colors`. */
export function toneColor(role: ToneRole, colors: ThemeColors): Oklch {
  const { canvas } = colors;
  switch (role) {
    case 'file':
    case 'unread':
    case 'recent':
    case 'suspect':
    case 'lit':
      return canvas[role];
    case 'scale1':
      return canvas.scale[0];
    case 'scale2':
      return canvas.scale[1];
    case 'scale3':
      return canvas.scale[2];
    case 'scale4':
      return canvas.scale[3];
  }
}

/** A tone with no files yet; the encoders fill in its count. */
export type ToneSpec = Omit<Tone, 'count'>;

/**
 * Sorts every file of `scene` into `specs` by `toneIndex` (an index into `specs`) and counts each
 * tone, keeping the order of `specs`. A tone no file shows is left out unless `keepEmpty`, for a
 * view whose zero is itself an answer ("no file read in the last 24 hours").
 */
export function sortIntoTones(
  scene: Scene,
  specs: readonly ToneSpec[],
  toneIndex: (id: number) => number,
  keepEmpty: boolean,
): Pick<ViewEncoding, 'toneOf' | 'tones'> {
  const fileCount = scene.snapshot.files.length;
  const counts = new Array<number>(specs.length).fill(0);
  const raw = new Uint8Array(fileCount);
  for (let id = 0; id < fileCount; id += 1) {
    const index = toneIndex(id);
    raw[id] = index;
    counts[index] = (counts[index] ?? 0) + 1;
  }
  const kept: Tone[] = [];
  const renumber = new Uint8Array(specs.length);
  for (const [index, spec] of specs.entries()) {
    const count = counts[index] ?? 0;
    if (count === 0 && !keepEmpty) continue;
    renumber[index] = kept.length;
    kept.push({ ...spec, count });
  }
  const toneOf = raw.map((index) => renumber[index] ?? 0);
  return { toneOf, tones: kept };
}
