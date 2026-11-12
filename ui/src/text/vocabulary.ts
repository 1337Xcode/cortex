// The map's words: one name per concept, used by the legend, the panels, the summary and the
// explanations shown on hover and focus, so the same thing is always called the same. Sentences
// that carry a number say what the number counts.
import type { GraphFile, GraphLink, LinkKind } from '../api/graph';
import type { FileMark } from '../map/fileEncoding';
import { countOf, formatCount } from './format';

/** What the picture is, said once on first open and again from the legend. */
export const INTRO_SENTENCE =
  'Folders are squares, files are circles on their rings; lines join files that change together.';

/** Each kind of learned link, as the legend and the panels name it. */
export const LINK_KIND_NAMES: Readonly<Record<LinkKind, string>> = {
  co_change: 'Changed together',
  used_together: 'Used together',
  work_order: 'Named in a work order',
};

/** One line on what each kind of link means, for the legend. */
export const LINK_KIND_MEANINGS: Readonly<Record<LinkKind, string>> = {
  co_change: 'Solid: the two files changed in the same commits.',
  used_together: 'Dashed: agents used both files in one task.',
  work_order: 'Dotted: Cortex named both files in one work order.',
};

/** Each mark a file's circle can carry, as the legend names it. */
export const MARK_NAMES: Readonly<Record<FileMark, string>> = {
  suspect: 'Has a suspect fact',
  recent: 'Read or edited in the last 24 hours',
  unread: 'Never read by an agent',
  plain: 'Other files',
};

/** One line on how each mark is drawn, for the legend: shape first, colour confirms it. */
export const MARK_MEANINGS: Readonly<Record<FileMark, string>> = {
  suspect: 'A second ring, in amber.',
  recent: 'A filled centre, in green.',
  unread: 'A dimmed, thinner outline.',
  plain: 'A plain grey circle.',
};

/** The folder marks, as the legend names them. */
export const FOLDER_NAMES = {
  folder: 'Folders',
  collapsed: 'Folders drawn whole',
} as const;

/** One line on each folder mark, for the legend. */
export const FOLDER_MEANINGS = {
  folder: 'A square; its files sit on rings around it.',
  collapsed: 'Zoomed out, a folder is one disc named with its file count.',
} as const;

/** The numbers of a file, as the panel and the explanations name them. */
export const NUMBER_NAMES = {
  lines: 'Lines',
  reads: 'Agent reads',
  commits: 'Commits',
  tokens: 'Tokens attributed',
} as const;

/** What the attributed tokens count. */
export const TOKENS_ATTRIBUTED_MEANING =
  'Tokens of agent turns whose last read or edit was this file.';

/** A file's name: the last segment of its path. */
function nameOf(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}

/** A folder's label on the canvas: its name and, drawn whole, its file count ("orders · 25 files"). */
export function folderLabel(name: string, fileCount: number | null): string {
  return fileCount === null ? name : `${name} · ${countOf(fileCount, 'file')}`;
}

/** What a file's circle says, in one sentence with its numbers, for hover. */
export function fileSentence(file: GraphFile, mark: FileMark): string {
  const parts = [
    `${nameOf(file.path)}: ${countOf(file.lines, 'line')}`,
    `read ${countOf(file.reads, 'time')} by agents`,
    `changed in ${countOf(file.commits, 'commit')}`,
  ];
  const state = mark === 'plain' ? '' : ` ${MARK_NAMES[mark]}.`;
  return `${parts.join(', ')}.${state}`;
}

/** What a folder drawn whole says, for hover. */
export function folderSentence(path: string, fileCount: number): string {
  return `${path === '' ? 'The repository' : `${path}/`}: ${countOf(fileCount, 'file')}. Click to open it.`;
}

/**
 * What a link says, in one sentence with its number, seen from file `from`: "changed with
 * service.py in 12 of 40 commits" counts the commits both files share against all of `from`'s.
 */
export function linkSentence(link: GraphLink, from: GraphFile, to: GraphFile): string {
  const [a, b] = [nameOf(from.path), nameOf(to.path)];
  switch (link.kind) {
    case 'co_change':
      return `${a} changed with ${b} in ${formatCount(link.commits)} of ${countOf(from.commits, 'commit')}.`;
    case 'used_together':
      return `${a} and ${b} were used together by agents, strength ${link.weight.toFixed(2)} of 1.`;
    case 'work_order':
      return `${a} and ${b} were named in the same work orders, strength ${link.weight.toFixed(2)} of 1.`;
  }
}
