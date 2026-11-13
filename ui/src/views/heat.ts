// The Heat view: the hotspots, by the tokens attributed to each file (the tokens of agent turns
// whose last read or edit was the file). Files with tokens fall into four steps of the accent on a
// log scale from the fewest to the most any file has, as token counts spread over orders of
// magnitude; files with none are dim. A folder drawn whole takes its typical file's step: with
// tokens spread over every folder, its hottest file would make every folder hot.
//
// The step of v tokens, from the fewest (lo) to the most (hi): ceil(4 x (ln v - ln lo) / (ln hi -
// ln lo)), at least 1. With lo 900 and hi 29,910: 2,600 tokens gives ceil(4 x 1.061 / 3.504) =
// ceil(1.21) = 2.
import { countOf, formatCount } from '../text/format';
import { noneClause, positiveRange, SCALE_STEPS, scaleTones } from './scaleTones';
import { defineView } from './view';

/** The step of `value` on a log scale from `min` to `max`, or 0 for none. */
export function heatStep(value: number, min: number, max: number): number {
  if (value <= 0) return 0;
  if (max <= min) return SCALE_STEPS;
  const share = (Math.log(value) - Math.log(min)) / (Math.log(max) - Math.log(min));
  return Math.min(SCALE_STEPS, Math.max(1, Math.ceil(SCALE_STEPS * share)));
}

/** The Heat view. */
export const heatView = defineView({
  name: 'heat',
  title: 'Heat',
  question: 'Where have agents spent the most tokens?',
  encode: (scene) => {
    const { files } = scene.snapshot;
    const values = Float64Array.from(files, (file) => file.tokens);
    const range = positiveRange(values);
    const sorted = scaleTones(
      scene,
      values,
      (value) => (range === null ? 0 : heatStep(value, range.min, range.max)),
      { key: 'heat', amount: (value) => countOf(value, 'token'), noneName: 'No tokens attributed' },
    );
    const none = files.filter((file) => file.tokens === 0).length;
    const scale =
      range === null
        ? 'No tokens have been attributed to any file yet.'
        : `Tokens attributed (input and output, cache reads included), on a log scale from ${formatCount(range.min)} to ${formatCount(range.max)}${noneClause(none)}`;
    return { ...sorted, isTinted: true, folderRule: 'typical', scale };
  },
});
