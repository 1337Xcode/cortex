// The tones of a view that ranks files on the sequential scale (Heat, Coupling): four steps of the
// accent, strongest first, each named by the smallest and largest value a file in it has, and a
// dim tone for files with nothing to rank. Where a file falls is the view's own rule (`stepOf`).
import type { Scene } from '../map/scene';
import { countOf, formatCount } from '../text/format';
import { sortIntoTones, type ToneRole, type ToneSpec, type ViewEncoding } from './view';

/** The steps of the scale; step 0 is "nothing to rank". */
export const SCALE_STEPS = 4;

/** The steps, strongest first, each with the colour role it draws in. */
const STEPS: readonly (readonly [number, ToneRole])[] = [
  [4, 'scale4'],
  [3, 'scale3'],
  [2, 'scale2'],
  [1, 'scale1'],
];

/** How a scale view names its values. */
export interface ScaleWords {
  /** The view's name, which keys its tones (`heat-4`, `heat-none`). */
  readonly key: string;
  /** A value with its unit: "4,100 tokens". */
  readonly amount: (value: number) => string;
  /** The dim tone's name: "No tokens attributed". */
  readonly noneName: string;
}

/** Names a step by the range of its values: "2,600 to 4,100 tokens", or one value alone. */
function rangeName(low: number, high: number, words: ScaleWords): string {
  return low === high ? words.amount(high) : `${formatCount(low)} to ${words.amount(high)}`;
}

/**
 * Sorts every file into a step of the scale by `stepOf(value)`, 0 to `SCALE_STEPS`, where
 * `values[id]` is the file's value; the strongest step comes first in the legend.
 */
export function scaleTones(
  scene: Scene,
  values: Float64Array,
  stepOf: (value: number) => number,
  words: ScaleWords,
): Pick<ViewEncoding, 'toneOf' | 'tones'> {
  const steps = Uint8Array.from(values, (value) => stepOf(value));
  const low = new Array<number>(SCALE_STEPS + 1).fill(Infinity);
  const high = new Array<number>(SCALE_STEPS + 1).fill(-Infinity);
  for (const [id, step] of steps.entries()) {
    const value = values[id] ?? 0;
    low[step] = Math.min(low[step] ?? Infinity, value);
    high[step] = Math.max(high[step] ?? -Infinity, value);
  }
  const specs: ToneSpec[] = [];
  for (const [step, role] of STEPS) {
    specs.push({
      key: `${words.key}-${step}`,
      role,
      isDim: false,
      rank: step,
      name: rangeName(low[step] ?? 0, high[step] ?? 0, words),
      meaning: '',
      mark: null,
    });
  }
  specs.push({
    key: `${words.key}-none`,
    role: 'unread',
    isDim: true,
    rank: 0,
    name: words.noneName,
    meaning: 'A dim, thin outline.',
    mark: null,
  });
  // Step s sits at index SCALE_STEPS - s; step 0 (nothing to rank) is last.
  return sortIntoTones(
    scene,
    specs,
    (id) => {
      const step = steps[id] ?? 0;
      return step === 0 ? SCALE_STEPS : SCALE_STEPS - step;
    },
    false,
  );
}

/** The end of a scale view's sentence: how many files have nothing to rank, when any have none. */
export function noneClause(none: number): string {
  return none === 0 ? '.' : `; ${countOf(none, 'file')} with none.`;
}

/** The smallest and largest positive value, or null when no value is positive. */
export function positiveRange(values: Float64Array): { min: number; max: number } | null {
  let min = Infinity;
  let max = -Infinity;
  for (const value of values) {
    if (value <= 0) continue;
    min = Math.min(min, value);
    max = Math.max(max, value);
  }
  return max < 0 ? null : { min, max };
}
