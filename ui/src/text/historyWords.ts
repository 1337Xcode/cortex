// The words of a file's "Commits behind its lines": what the list is (the newest of how many), and
// why it is empty, by whether the history worker runs. With the worker off, no file ever gets a
// history, and the panel says so and how to turn it on instead of promising one.
import { MAX_HISTORY, type FileDetail } from '../api/file';
import { countOf, formatCount } from './format';

/** The section's title: the commits that last changed the file's current lines, not its commits. */
export const HISTORY_TITLE = 'Commits behind its lines';

/** What the section says above its list, or, with nothing listed, instead of it. */
export function historyNote(
  detail: Pick<FileDetail, 'history' | 'history_total' | 'history_reader'>,
): string {
  const total = detail.history_total;
  if (total > 0) {
    return total > detail.history.length
      ? `The ${formatCount(detail.history.length)} newest of ${countOf(total, 'commit')} behind its current lines.`
      : `The ${countOf(total, 'commit')} behind its current lines, newest first.`;
  }
  switch (detail.history_reader) {
    case 'off':
      return "Cortex reads the commits behind a file's lines only when history.why or history.seeds is on, and both are off, so no file has them. To read them, run cortex config set history.why true.";
    case 'refused':
      return "Git is refused in this repository, so Cortex reads no commits behind any file's lines.";
    case 'on':
      return `No commit is stored for this file's current lines yet. Cortex reads them when a prompt or its upkeep asks about the file, and lists the ${formatCount(MAX_HISTORY)} newest.`;
  }
}
