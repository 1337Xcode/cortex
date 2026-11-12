// The Fog view: what agents have explored. Files an agent has read are lit; files no agent has
// read are in fog, dim and thin. A folder drawn whole is lit when any of its files is, so a dim one
// is a folder no agent has looked into. A repository
// no agent has worked in is all fog, and the legend says so: the cold map is meant to look like this.
import { countOf } from '../text/format';
import { defineView, sortIntoTones, type ToneSpec } from './view';

const LIT: ToneSpec = {
  key: 'fog-lit',
  role: 'lit',
  isDim: false,
  rank: 1,
  name: 'Read by an agent',
  meaning: 'Lit: a bright outline and a light fill.',
  mark: null,
};

const FOG: ToneSpec = {
  key: 'fog-dim',
  role: 'unread',
  isDim: true,
  rank: 0,
  name: 'Never read',
  meaning: 'In fog: a dim, thin outline.',
  mark: null,
};

/** The Fog view. */
export const fogView = defineView({
  name: 'fog',
  title: 'Fog',
  question: 'What have agents explored, and what have they never looked at?',
  encode: (scene) => {
    const { files } = scene.snapshot;
    const sorted = sortIntoTones(
      scene,
      [LIT, FOG],
      (id) => ((files[id]?.reads ?? 0) > 0 ? 0 : 1),
      true,
    );
    const read = sorted.tones[0]?.count ?? 0;
    const scale =
      read === 0
        ? 'No agent has read a file here yet, so the whole map is in fog.'
        : `Agents have read ${countOf(read, 'file')} of ${files.length}; the other ${files.length - read} are in fog.`;
    return { ...sorted, isTinted: true, folderRule: 'strongest', scale };
  },
});
