// The summary's sentences, in plain words with numbers from the repository: what is indexed, how
// much agents have read, the strongest coupled pair and the busiest folder. A cold repository gets
// one sentence saying the map shows its structure alone. Each sentence has a short form for the
// one-line strip. Ids the summary names are checked against the snapshot here, so a summary from
// another moment of the store is refused, not shown: the strongest pair must be one of the map's
// co-change links, with the same commits.
import type { GraphSnapshot } from '../api/graph';
import type { ApiError } from '../api/guard';
import type { MapSummary } from '../api/summary';
import { countOf, formatCount, formatPercent } from '../text/format';

/** A piece of a sentence: plain text, or a file the page renders as a button that flies to it. */
export type SentencePart = string | { readonly file: number; readonly path: string };

/** One sentence, optionally naming files the person can jump to. */
export interface SummarySentence {
  readonly key: 'indexed' | 'cold' | 'explored' | 'pair' | 'busiest';
  /** The sentence in full. */
  readonly parts: readonly SentencePart[];
  /** The same sentence shorter, for the one-line strip when the full one does not fit. */
  readonly short: readonly SentencePart[];
}

/** The sentences, or the reason the summary cannot be shown with this snapshot. */
export type SummaryText =
  | { readonly ok: true; readonly sentences: readonly SummarySentence[] }
  | { readonly ok: false; readonly error: ApiError };

function fileRef(snapshot: GraphSnapshot, id: number): { file: number; path: string } | null {
  const file = snapshot.files[id];
  return file === undefined ? null : { file: id, path: file.path };
}

function pairSentence(
  summary: MapSummary,
  snapshot: GraphSnapshot,
): SummarySentence | null | 'bad' {
  const pair = summary.strongest_pair;
  if (pair === null) return null;
  const from = fileRef(snapshot, pair.from);
  const to = fileRef(snapshot, pair.to);
  const isLink = snapshot.links.some(
    (link) =>
      link.kind === 'co_change' &&
      link.from === pair.from &&
      link.to === pair.to &&
      link.commits === pair.commits,
  );
  if (from === null || to === null || !isLink) return 'bad';
  const commits = countOf(pair.commits, 'commit');
  return {
    key: 'pair',
    parts: ['Strongest coupling: ', from, ' and ', to, ` changed together in ${commits}.`],
    short: [from, ' and ', to, `: ${commits} together.`],
  };
}

function exploredSentence(summary: MapSummary): SummarySentence {
  const read = summary.explored_files;
  if (read === 0) {
    return {
      key: 'explored',
      parts: ['Agents have not read any of these files yet.'],
      short: ['No file read by agents yet.'],
    };
  }
  const share = formatPercent(read, summary.files);
  return {
    key: 'explored',
    parts: [
      `Agents have read ${formatCount(read)} of the ${countOf(summary.files, 'file')} (${share}).`,
    ],
    short: [`${share} read by agents.`],
  };
}

function busiestSentence(summary: MapSummary): SummarySentence | null {
  const busiest = summary.busiest_folder;
  if (busiest === null) return null;
  const window =
    summary.window_days === 1 ? 'the last day' : `the last ${summary.window_days} days`;
  const tokens = `${countOf(busiest.tokens, 'token')} attributed`;
  return {
    key: 'busiest',
    parts: [`Most worked on in ${window}: ${busiest.path}/ (${tokens}).`],
    short: [`Busiest folder: ${busiest.path}/.`],
  };
}

/** Builds the sentences for `summary`, checked against `snapshot`, most useful first. */
export function summarySentences(summary: MapSummary, snapshot: GraphSnapshot): SummaryText {
  const files = countOf(summary.files, 'file');
  const indexed: SummarySentence = {
    key: 'indexed',
    parts: [`${files} in ${countOf(summary.folders, 'folder')}.`],
    short: [`${files}.`],
  };
  if (!summary.learned) {
    const cold: SummarySentence = {
      key: 'cold',
      parts: ['Cortex has not learned anything here yet, so the map shows the structure alone.'],
      short: ['Nothing learned yet: the structure alone.'],
    };
    return { ok: true, sentences: [cold, indexed] };
  }
  const pair = pairSentence(summary, snapshot);
  if (pair === 'bad') {
    const message = 'the summary names a pair of files the map does not link';
    return { ok: false, error: { code: 'bad_range', message } };
  }
  // Most useful first: the strip shows one line, and the first sentence is the one always seen.
  const sentences = [pair, busiestSentence(summary), exploredSentence(summary), indexed];
  return { ok: true, sentences: sentences.filter((sentence) => sentence !== null) };
}
