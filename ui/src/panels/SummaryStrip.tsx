// The strip of plain sentences at the top of the map: what is indexed and what Cortex has learned,
// with the strongest pair's files as buttons that fly to them. Its loading and error states say so
// in words; a cold repository says the map shows the structure alone.
import { useId, useState, type ReactElement } from 'react';
import type { GraphSnapshot } from '../api/graph';
import { useMapStore, type SummaryState } from '../state/store';
import { distinctNames } from '../text/names';
import { summarySentences, type SummarySentence } from './summarySentences';

function Sentence({ sentence }: { sentence: SummarySentence }): ReactElement {
  const selectFile = useMapStore((state) => state.selectFile);
  const paths = sentence.parts.flatMap((part) => (typeof part === 'string' ? [] : [part.path]));
  const names = distinctNames(paths);
  let fileIndex = 0;
  return (
    <li className="summary-sentence" data-testid={`summary-${sentence.key}`}>
      {sentence.parts.map((part, index) =>
        typeof part === 'string' ? (
          <span key={index}>{part}</span>
        ) : (
          <button
            key={index}
            type="button"
            className="link-button"
            title={part.path}
            onClick={() => {
              selectFile(part.file, { fly: true });
            }}
          >
            {names[fileIndex++] ?? part.path}
          </button>
        ),
      )}
    </li>
  );
}

/**
 * The sentences on one line, the most useful first; the rest are cut with an ellipsis on screen
 * (screen readers still read them all) until the person asks for all of them.
 */
function SentenceList({ sentences }: { sentences: readonly SummarySentence[] }): ReactElement {
  const [isOpen, setIsOpen] = useState(false);
  const listId = useId();
  return (
    <>
      <ul className="summary-sentences" id={listId} data-open={isOpen}>
        {sentences.map((sentence) => (
          <Sentence key={sentence.key} sentence={sentence} />
        ))}
      </ul>
      {sentences.length > 1 ? (
        <button
          type="button"
          className="summary-toggle"
          aria-expanded={isOpen}
          aria-controls={listId}
          onClick={() => {
            setIsOpen(!isOpen);
          }}
        >
          {isOpen ? 'Show less' : 'Show all'}
        </button>
      ) : null}
    </>
  );
}

function stripBody(summary: SummaryState, snapshot: GraphSnapshot): ReactElement {
  if (summary.status === 'loading') return <p className="summary-state">Loading the summary.</p>;
  const text =
    summary.status === 'error'
      ? { ok: false as const, error: summary.error }
      : summarySentences(summary.summary, snapshot);
  if (!text.ok) {
    return (
      <p className="summary-state" role="alert">
        The summary could not be loaded ({text.error.code}): {text.error.message}
      </p>
    );
  }
  return <SentenceList sentences={text.sentences} />;
}

/** The summary strip for the loaded snapshot. */
export function SummaryStrip({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const summary = useMapStore((state) => state.summary);
  return (
    <section className="summary" aria-label="Summary">
      {stripBody(summary, snapshot)}
    </section>
  );
}
