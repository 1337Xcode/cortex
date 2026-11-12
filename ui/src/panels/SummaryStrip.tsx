// The strip of plain sentences at the top of the map, kept to one line: the most useful sentences
// that fit whole, in full or short, with the files they name as buttons that fly to them. Every
// sentence is also in the overview panel's "At a glance", so nothing the strip leaves out is lost.
// Its loading and error states say so in words.
import {
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
  type RefObject,
} from 'react';
import type { GraphSnapshot } from '../api/graph';
import { ClippedText } from '../components/Clipped';
import { Button } from '../components/ui/button';
import { useFontsSettled } from '../fonts/fonts';
import { useMapStore, type SummaryState } from '../state/store';
import { fontMeasurer } from '../text/measure';
import { distinctNames } from '../text/names';
import { SPACE } from '../theme/tokens';
import { fitStrip, type StripLine } from './stripFit';
import {
  summarySentences,
  type SentencePart,
  type SummarySentence,
  type SummaryText,
} from './summarySentences';

/** The names a sentence shows for the files it names, told apart from each other. */
function namesOf(parts: readonly SentencePart[]): string[] {
  return distinctNames(parts.flatMap((part) => (typeof part === 'string' ? [] : [part.path])));
}

/** A sentence's text as the page shows it, file names in place of the files. */
export function sentenceText(parts: readonly SentencePart[]): string {
  const names = namesOf(parts);
  let fileIndex = 0;
  return parts
    .map((part) => (typeof part === 'string' ? part : (names[fileIndex++] ?? part.path)))
    .join('');
}

/** A sentence, its files as buttons that fly to them. */
export function SentenceView({ parts }: { parts: readonly SentencePart[] }): ReactElement {
  const selectFile = useMapStore((state) => state.selectFile);
  const names = namesOf(parts);
  let fileIndex = 0;
  return (
    <>
      {parts.map((part, index) =>
        typeof part === 'string' ? (
          <span key={index}>{part}</span>
        ) : (
          <Button
            key={index}
            variant="link"
            size="inline"
            className="sentence-file"
            title={part.path}
            onClick={() => {
              selectFile(part.file, { fly: true });
            }}
          >
            {names[fileIndex++] ?? part.path}
          </Button>
        ),
      )}
    </>
  );
}

/** The line the strip shows for `sentences` at the width of `ref`'s element, refitted on resize. */
function useStripLine(
  ref: RefObject<HTMLElement | null>,
  sentences: readonly SummarySentence[],
): StripLine | null {
  const [line, setLine] = useState<StripLine | null>(null);
  const isSettled = useFontsSettled();
  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null || !isSettled) return undefined;
    const fit = (): void => {
      const font = getComputedStyle(element).font;
      const measure = fontMeasurer();
      const text = { textOf: sentenceText, measure: (value: string) => measure(value, font) };
      setLine(fitStrip(sentences, element.clientWidth, SPACE.lg, text));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [ref, sentences, isSettled]);
  return line;
}

/** The summary's sentences, or its error; null while it loads. */
export function summaryText(summary: SummaryState, snapshot: GraphSnapshot): SummaryText | null {
  if (summary.status === 'loading') return null;
  if (summary.status === 'error') return { ok: false, error: summary.error };
  return summarySentences(summary.summary, snapshot);
}

/** The sentences that fit, on one line. */
function StripSentences({ sentences }: { sentences: readonly SummarySentence[] }): ReactElement {
  const ref = useRef<HTMLDivElement>(null);
  const line = useStripLine(ref, sentences);
  return (
    <div className="summary-line" ref={ref}>
      {line === null ? null : line.kind === 'cut' ? (
        <ClippedText text={line.text} kind="words" />
      ) : (
        <ul className="summary-sentences">
          {line.sentences.map((sentence) => (
            <li key={sentence.key} data-strip={sentence.key}>
              <SentenceView parts={sentence.parts} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/** The summary strip for the loaded snapshot. */
export function SummaryStrip({ snapshot }: { snapshot: GraphSnapshot }): ReactElement {
  const summary = useMapStore((state) => state.summary);
  // One array per summary and snapshot, so the strip refits only when they change.
  const text = useMemo(() => summaryText(summary, snapshot), [summary, snapshot]);
  return (
    <section className="summary" aria-label="Summary">
      {text === null ? (
        <p className="summary-state">Loading the summary.</p>
      ) : text.ok ? (
        <StripSentences sentences={text.sentences} />
      ) : (
        <p className="summary-state" role="alert">
          <ClippedText
            text={`The summary could not be loaded (${text.error.code}): ${text.error.message}.`}
            kind="words"
          />
        </p>
      )}
    </section>
  );
}
