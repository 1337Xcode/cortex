// A row of two lines for a list whose rows say a sentence (the activity, the warnings): its icons,
// then what happened over its details, both wrapping, never cut. A row about a file the map lists
// is a native button that flies to the file and says its whole sentence on hover and focus; any
// other row is text, its whole sentence shown on hover and read in full by screen readers.
import type { ReactElement, ReactNode } from 'react';
import { Tooltip, TooltipContent, TooltipTrigger } from '../components/ui/tooltip';
import { useMapStore } from '../state/store';

/** A row that flies to `file`, or a row of text when `file` is null. */
export function StackRow({
  file,
  lead,
  what,
  meta,
  sentence,
  state = null,
}: {
  file: number | null;
  /** The icons before the text, hidden from screen readers (the sentence says what they show). */
  lead: ReactNode;
  /** What happened, the row's first line. */
  what: string;
  /** Its details, the second line: each part kept whole on one line, parts joined by " · ". */
  meta: readonly string[];
  /** The whole row as a sentence: its accessible name and its tooltip. */
  sentence: string;
  /** A state the stylesheet marks, such as a warning still open. */
  state?: string | null;
}): ReactElement {
  const selectFile = useMapStore((store) => store.selectFile);
  const body = (
    <>
      <span className="agent-lead" aria-hidden="true">
        {lead}
      </span>
      <span className="stack-text">
        <span className="stack-what">{what}</span>
        <span className="stack-meta">
          {meta.map((part, index) => (
            <span key={index}>
              {index === 0 ? '' : ' · '}
              <span className="stack-meta-part">{part}</span>
            </span>
          ))}
        </span>
      </span>
    </>
  );
  if (file === null) {
    return (
      <p className="stack-row" data-state={state ?? undefined} title={sentence}>
        <span aria-hidden="true" className="contents">
          {body}
        </span>
        <span className="visually-hidden">{sentence}</span>
      </p>
    );
  }
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          className="stack-row"
          data-state={state ?? undefined}
          aria-label={sentence}
          onClick={() => {
            selectFile(file, { fly: true });
          }}
        >
          {body}
        </button>
      </TooltipTrigger>
      <TooltipContent>{sentence}</TooltipContent>
    </Tooltip>
  );
}
