// The words of the person's say on a fact: which word each fact's status offers, what the button
// says while it waits, what the retire step asks, and what came of the word, said in full.
import type { FactStatus } from '../api/file';
import type { FactWord, FactWordOutcome } from '../api/factWord';
import { visibleText } from '../api/guard';

/** What a fact's status is called in its row. */
export const STATUS_NAMES: Readonly<Record<FactStatus, string>> = {
  active: 'Active',
  candidate: 'Candidate',
  suspect: 'Suspect',
};

/**
 * The words a fact in `status` offers, in the order they show: a candidate is confirmed; an active
 * fact is retired; a suspect fact (its code changed since) is confirmed again, which says it still
 * holds for the code as it is now, or retired.
 */
export function wordsFor(status: FactStatus): readonly FactWord[] {
  switch (status) {
    case 'candidate':
      return ['confirm'];
    case 'active':
      return ['retire'];
    case 'suspect':
      return ['confirm', 'retire'];
  }
}

/** A word's button: "Confirm", "Confirm again" for a suspect fact, "Retire". */
export function wordLabel(word: FactWord, status: FactStatus): string {
  if (word === 'retire') return 'Retire';
  return status === 'suspect' ? 'Confirm again' : 'Confirm';
}

/** A word's button while the daemon writes it. */
export const WAITING_LABEL: Readonly<Record<FactWord, string>> = {
  confirm: 'Confirming…',
  retire: 'Retiring…',
};

/** The question the retire step asks before the word is sent. */
export function retireQuestion(id: number): string {
  return `Retire fact ${id}? Cortex will stop showing it here and stop giving it to agents.`;
}

/** The retire step's two buttons. */
export const RETIRE_CHOICES = { yes: 'Retire it', no: 'Keep it' } as const;

/** Why the daemon did not say `word` on fact `id` was written, with its reason in its words. */
function undoneSentence(
  id: number,
  word: FactWord,
  outcome: Extract<FactWordOutcome, { kind: 'undone' }>,
): string {
  const reason = visibleText(outcome.reason);
  const done = word === 'confirm' ? 'confirmed' : 'retired';
  switch (outcome.why) {
    case 'refused':
      return `Cortex refused to ${word} fact ${id}: ${reason}.`;
    case 'unwritten':
      return `Fact ${id} was not ${done}: ${reason}.`;
    case 'busy':
      return `Cortex did not say fact ${id} was ${done}: ${reason}.`;
  }
}

/** What happened to fact `id` after `word`, in one sentence. */
export function outcomeSentence(id: number, word: FactWord, outcome: FactWordOutcome): string {
  switch (outcome.kind) {
    case 'done':
      return outcome.status === 'active'
        ? `Fact ${id} confirmed: it is active now, and Cortex may give it to agents when its evidence passes Cortex's checks.`
        : `Fact ${id} retired: Cortex no longer shows it or gives it to agents.`;
    case 'undone':
      return undoneSentence(id, word, outcome);
    case 'unanswered':
      return `No answer came to the word on fact ${id} (${outcome.error.code}): ${visibleText(outcome.error.message)}. Cortex may have stopped; if it runs, reload to see whether it was done.`;
    case 'unread':
      return `Cortex's answer on fact ${id} could not be read (${outcome.error.code}): ${visibleText(outcome.error.message)}. Reload to see whether it was done.`;
  }
}
