// The default view, the map's own look: each file's outline shows its most pressing mark. A
// suspect fact (amber, with a second ring), read or edited in the last 24 hours (green, with a
// filled centre), never read while others were (dim and thin), or nothing (neutral grey). Fills
// and folders stay neutral.
import { fileMark, type FileMark } from '../map/fileEncoding';
import { MARK_MEANINGS, MARK_NAMES } from '../text/vocabulary';
import { defineView, sortIntoTones, type ToneMark, type ToneRole, type ToneSpec } from './view';

/** The marks in the legend's order, each with its colour role and its second channel. */
const MARKS: readonly (readonly [FileMark, ToneRole, ToneMark])[] = [
  ['recent', 'recent', 'dot'],
  ['suspect', 'suspect', 'ring'],
  ['unread', 'unread', null],
  ['plain', 'file', null],
];

/** The default view. */
export const defaultView = defineView({
  name: 'default',
  title: 'Default',
  question: 'Which files carry a mark: suspect, recent or never read?',
  encode: (scene, nowMs) => {
    // "Never read" says something only once some file has been read.
    const marks = MARKS.filter(([mark]) => mark !== 'unread' || scene.hasReads);
    const specs: ToneSpec[] = marks.map(([mark, role, second]) => ({
      key: mark,
      role,
      isDim: mark === 'unread',
      rank: 0,
      name: MARK_NAMES[mark],
      meaning: MARK_MEANINGS[mark],
      mark: second,
    }));
    const order = marks.map(([mark]) => mark);
    const { files } = scene.snapshot;
    const toneIndex = (id: number): number => {
      const file = files[id];
      return order.indexOf(file === undefined ? 'plain' : fileMark(file, nowMs, scene.hasReads));
    };
    return {
      ...sortIntoTones(scene, specs, toneIndex, true),
      isTinted: false,
      folderRule: 'strongest',
      scale: 'Each file shows its most pressing mark: shape first, colour confirms it.',
    };
  },
});
