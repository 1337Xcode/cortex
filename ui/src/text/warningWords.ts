// The words of the warnings Cortex gave agents after their edits: what each said, in words by its
// kind ("book.rs usually changes with mod.rs", "tests/orders.rs is a test of book.rs", "app.ts
// calls a symbol of book.rs whose signature changed"), its strength as the daemon means it for that
// kind, and what became of it.
import type { GivenWarning, WarningKind, WarningState } from '../api/warnings';
import { formatDate, formatUnit } from './format';

/** What a warning's state is called, said in a row. */
export const STATE_NAMES: Readonly<Record<WarningState, string>> = {
  open: 'Open',
  followed: 'Followed',
  not_followed: 'Not followed',
};

/** What each state means, said in the row's sentence. */
const STATE_MEANINGS: Readonly<Record<WarningState, string>> = {
  open: 'open: its task has not ended yet',
  followed: 'followed: the named file was read or edited after it',
  not_followed: 'not followed: its task ended without reading or editing the named file',
};

/**
 * What a warning's strength measures, by its kind. A test or call-site warning can name several
 * files and keeps one strength for all of them, the strongest, so it is not the named file's own.
 */
export const STRENGTH_MEANINGS: Readonly<Record<WarningKind, string>> = {
  cochange: 'how often the two files changed, or were worked on, together',
  test: 'how sure Cortex is that a test file tests the edited one, the strongest of the files the warning named',
  call_site:
    'how firmly the index ties a call to the changed symbol, the strongest of the files the warning named',
};

/**
 * What a warning said, by kind, naming the edited file `source` and the named file `target` (a
 * name the map no longer lists reads "a file the map no longer lists").
 */
export function warningLine(
  kind: WarningKind,
  source: string | null,
  target: string | null,
): string {
  const edited = source ?? 'a file the map no longer lists';
  const named = target ?? 'a file the map no longer lists';
  switch (kind) {
    case 'cochange':
      return `${edited} usually changes with ${named}`;
    case 'test':
      return `${named} is a test of ${edited}`;
    case 'call_site':
      return `${named} calls a symbol of ${edited} whose signature changed`;
  }
}

/** A strength from 0 to 1 as the rows write it: "0.62", never a false "0.00" or "1.00". */
export function strengthText(strength: number): string {
  return formatUnit(strength);
}

/** The row's second line, part by part: what became of it, its strength and its date. */
export function warningMeta(warning: GivenWarning): readonly string[] {
  return [
    STATE_NAMES[warning.state],
    `strength ${strengthText(warning.strength)}`,
    formatDate(warning.at_ms),
  ];
}

/** The whole warning in sentences, for its row's accessible name and tooltip. */
export function warningSentence(
  warning: GivenWarning,
  source: string | null,
  target: string | null,
): string {
  const line = warningLine(warning.kind, source, target);
  return `${line}. Given ${formatDate(warning.at_ms)}; ${STATE_MEANINGS[warning.state]}. Strength ${strengthText(warning.strength)} of 1: ${STRENGTH_MEANINGS[warning.kind]}.`;
}

/** The warnings open first, then the resolved, each part in the daemon's order (newest first). */
export function openFirst(warnings: readonly GivenWarning[]): GivenWarning[] {
  return [
    ...warnings.filter((warning) => warning.state === 'open'),
    ...warnings.filter((warning) => warning.state !== 'open'),
  ];
}
