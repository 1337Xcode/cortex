// The Recent view: what changed lately. Files an agent read or edited in the 24 hours before the
// map last read its data (when it loaded, or its latest quiet refresh while agents work) are lit
// green; the rest are dimmed. A folder drawn whole is lit when any of its files is.
import { RECENT_WINDOW_MS } from '../theme/tokens';
import { countOf, formatCount } from '../text/format';
import { defineView, sortIntoTones, type ToneSpec } from './view';

const LIT: ToneSpec = {
  key: 'recent-lit',
  role: 'recent',
  isDim: false,
  rank: 1,
  name: 'Read or edited in the last 24 hours',
  meaning: 'Lit: a green outline and fill.',
  mark: null,
};

const DIM: ToneSpec = {
  key: 'recent-dim',
  role: 'unread',
  isDim: true,
  rank: 0,
  name: 'Not in the last 24 hours',
  meaning: 'A dim, thin outline.',
  mark: null,
};

/** The Recent view. */
export const recentView = defineView({
  name: 'recent',
  title: 'Recent',
  question: 'What did agents read or edit in the last 24 hours?',
  encode: (scene, nowMs) => {
    const { files } = scene.snapshot;
    const isRecent = (id: number): boolean => {
      const touched = files[id]?.touched_ms ?? null;
      return touched !== null && nowMs - touched <= RECENT_WINDOW_MS;
    };
    const sorted = sortIntoTones(scene, [LIT, DIM], (id) => (isRecent(id) ? 0 : 1), true);
    const lit = sorted.tones[0]?.count ?? 0;
    const scale =
      lit === 0
        ? 'No file was read or edited by an agent in the 24 hours before the map last read its data.'
        : `${countOf(lit, 'file')} of ${formatCount(files.length)} read or edited by an agent in the 24 hours before the map last read its data.`;
    return { ...sorted, isTinted: true, folderRule: 'strongest', scale };
  },
});
