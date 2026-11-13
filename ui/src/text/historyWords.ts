// The words of a file's history: what its "Commits behind its lines" list is (the newest of how
// many commits whose subjects Cortex holds), and why it is empty, by whether the history worker
// runs; and whether Cortex has read the repository's commit history at all, which every commit
// count rests on. With the worker off, no file ever gets a list, and the panel says so and how to
// turn it on instead of promising one. Until the history miner has read the repository, a file's
// commit count of 0 says nothing, so the page says why instead of showing it.
import { visibleText } from '../api/guard';
import { MAX_HISTORY, type FileDetail } from '../api/file';
import type { GitState, Health } from '../api/health';
import { countOf, formatCount } from './format';

/** The section's title: the commits that last changed the file's current lines, not its commits. */
export const HISTORY_TITLE = 'Commits behind its lines';

/** What the section says above its list, or, with nothing listed, instead of it. */
export function historyNote(
  detail: Pick<FileDetail, 'history' | 'history_total' | 'history_reader'>,
): string {
  const total = detail.history_total;
  if (total > 0) {
    const commits = `${countOf(total, 'commit')} behind its current lines whose subjects Cortex holds`;
    return total > detail.history.length
      ? `The ${formatCount(detail.history.length)} newest of the ${commits}.`
      : `The ${commits}, newest first.`;
  }
  switch (detail.history_reader) {
    case 'off':
      return "Cortex reads the commits behind a file's lines only when history.why or history.seeds is on, and both are off, so no file has them. To read them, run cortex config set history.why true.";
    case 'refused':
      return "Git is refused in this repository, so Cortex reads no commits behind any file's lines.";
    case 'on':
      return `No commit behind this file's current lines is held yet. Cortex reads them when a prompt or its upkeep asks about the file, and lists the ${formatCount(MAX_HISTORY)} newest.`;
  }
}

/**
 * Whether Cortex has read the repository's commit history, which every commit count is taken
 * from: read once the history miner is ready; otherwise a count of 0 is no answer, and the page
 * shows `value` in its place, `clause` in a sentence, and `why` to say what it means.
 */
export type HistoryRead =
  | { readonly kind: 'read' }
  | {
      readonly kind: 'unread';
      /** What the file panel shows for a count of 0 ("Not read yet"). */
      readonly value: string;
      /** The same in the middle of a sentence ("its commits not read yet"). */
      readonly clause: string;
      /** What it means, in a sentence. */
      readonly why: string;
    };

/** Not read yet, with `why`. */
function notReadYet(why: string): HistoryRead {
  return { kind: 'unread', value: 'Not read yet', clause: 'its commits not read yet', why };
}

/** Why the history is not read, by the miner's state other than ready. */
const UNREAD_WHY: Readonly<Record<Exclude<GitState, 'ready' | 'no_repository'>, string>> = {
  waiting:
    "Cortex has not read this repository's history yet: its history miner waits for the first index.",
  mining: "Cortex is still reading this repository's history.",
  failed:
    "Cortex could not read this repository's history; its history miner tries again after a pause, or when HEAD moves.",
  other:
    "Cortex has not read this repository's history: its history miner is in a state this page does not know.",
};

/**
 * What `health` (null until the daemon has answered) says of the repository's history: read only
 * when the daemon reports its miner ready. A health with no report of the daemon's says nothing,
 * so the history counts as not read.
 */
export function historyRead(health: Health | null): HistoryRead {
  if (health === null) {
    return notReadYet("Cortex has not said yet whether it has read this repository's history.");
  }
  if (health.daemon.status !== 'read') {
    return notReadYet("Cortex did not say whether it has read this repository's history.");
  }
  const { git, git_detail: detail } = health.daemon.health;
  if (git === 'ready') return { kind: 'read' };
  if (git === 'no_repository') {
    return {
      kind: 'unread',
      value: 'No history',
      clause: 'no history read (not a git repository)',
      why: "Cortex reads no history here: the repository's folder is not the top of a git repository.",
    };
  }
  const why = UNREAD_WHY[git];
  return notReadYet(detail === null ? why : `${why} ${visibleText(detail)}`);
}
