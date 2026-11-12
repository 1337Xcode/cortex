// The Coupling view: how tied each file is to the rest, by the number of learned links it has (of
// any kind and any strength, as the map holds them). Files with links fall into four steps of the
// accent, linear in the count up to the most any file has; files with none are dim. A folder
// drawn whole takes its typical file's step.
//
// The step of a file with c links, when the most is m: ceil(4 x c / m), at least 1. With m = 5,
// 2 links gives ceil(1.6) = 2 and 4 links ceil(3.2) = 4.
import { countOf } from '../text/format';
import { noneClause, positiveRange, SCALE_STEPS, scaleTones } from './scaleTones';
import { defineView } from './view';

/** The step of a file with `links` links when the most any file has is `max`, or 0 for none. */
export function couplingStep(links: number, max: number): number {
  if (links <= 0 || max <= 0) return 0;
  return Math.min(SCALE_STEPS, Math.max(1, Math.ceil((SCALE_STEPS * links) / max)));
}

/** The Coupling view. */
export const couplingView = defineView({
  name: 'coupling',
  title: 'Coupling',
  question: 'Which files are tied to the most others?',
  encode: (scene) => {
    const { offsets } = scene.adjacency;
    const fileCount = scene.snapshot.files.length;
    const values = new Float64Array(fileCount);
    for (let id = 0; id < fileCount; id += 1) {
      values[id] = (offsets[id + 1] ?? 0) - (offsets[id] ?? 0);
    }
    const range = positiveRange(values);
    const sorted = scaleTones(
      scene,
      values,
      (value) => (range === null ? 0 : couplingStep(value, range.max)),
      { key: 'coupling', amount: (value) => countOf(value, 'link'), noneName: 'No learned links' },
    );
    const none = values.filter((value) => value === 0).length;
    const scale =
      range === null
        ? 'No links have been learned yet.'
        : `Learned links per file, of any kind and strength, from ${range.min} to ${range.max}${noneClause(none)}`;
    return { ...sorted, isTinted: true, folderRule: 'typical', scale };
  },
});
