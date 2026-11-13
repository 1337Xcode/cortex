// The strip of plain sentences at the top of the map, kept to one line: the most useful sentences
// that fit whole, in full or short, with the files they name as buttons that fly to them (named
// as the map names its files, whole on hover). It reports which sentences it shows in full; the
// overview panel's "At a glance" holds the others, so each sentence is on the page once in full.
// Its loading and error states say so in words. A summary the snapshot on screen refuses (it names
// a pair of files the map does not link) comes from a newer moment of the store than the snapshot,
// so the strip asks the map to read its data again, as the activity and the warnings do.
import {
  useEffect,
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
import { namesOfFiles, type PathNames } from '../text/pathNames';
import { SPACE } from '../theme/tokens';
import { fitStrip, fullKeys, type StripLine } from './stripFit';
import {
  summarySentences,
  type SentencePart,
  type SummarySentence,
  type SummaryText,
} from './summarySentences';
import { whyNotLoaded } from '../text/loadWords';

/** A sentence's text as the page shows it, the files by the names `names` gives them. */
export function sentenceText(parts: readonly SentencePart[], names: PathNames): string {
  return parts.map((part) => (typeof part === 'string' ? part : names.nameOf(part.path))).join('');
}

/** A sentence, its files as buttons that fly to them, each with its whole path on hover. */
export function SentenceView({
  parts,
  names,
}: {
  parts: readonly SentencePart[];
  names: PathNames;
}): ReactElement {
  const selectFile = useMapStore((state) => state.selectFile);
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
            {names.nameOf(part.path)}
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
  names: PathNames,
): StripLine | null {
  const [line, setLine] = useState<StripLine | null>(null);
  const isSettled = useFontsSettled();
  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null || !isSettled) return undefined;
    const fit = (): void => {
      const font = getComputedStyle(element).font;
      const measure = fontMeasurer();
      const text = {
        textOf: (parts: readonly SentencePart[]) => sentenceText(parts, names),
        measure: (value: string) => measure(value, font),
      };
      setLine(fitStrip(sentences, element.clientWidth, SPACE.lg, text));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [ref, sentences, names, isSettled]);
  return line;
}

/** The summary's sentences, or its error; null while it loads. */
export function summaryText(summary: SummaryState, snapshot: GraphSnapshot): SummaryText | null {
  if (summary.status === 'loading') return null;
  if (summary.status === 'error') return { ok: false, error: summary.error };
  return summarySentences(summary.summary, snapshot);
}

/**
 * Whether the summary arrived but the snapshot on screen refuses it (`text` is what it made of
 * it): the two come from different moments of the store, and reading both again agrees them. A
 * summary that failed to load is not out of step.
 */
export function isSummaryOutOfStep(summary: SummaryState, text: SummaryText | null): boolean {
  return summary.status === 'ready' && text !== null && !text.ok;
}

/** The sentences that fit, on one line; `onFull` hears which of them it shows in full. */
function StripSentences({
  sentences,
  names,
  onFull,
}: {
  sentences: readonly SummarySentence[];
  names: PathNames;
  onFull: (keys: string) => void;
}): ReactElement {
  const ref = useRef<HTMLDivElement>(null);
  const line = useStripLine(ref, sentences, names);
  const full = line === null ? '' : fullKeys(line, sentences);
  // Before the first paint, so the overview never shows a sentence twice.
  useLayoutEffect(() => {
    onFull(full);
  }, [full, onFull]);
  const fullSet = new Set(full.split(' '));
  return (
    <div className="summary-line" ref={ref}>
      {line === null ? null : line.kind === 'cut' ? (
        <ClippedText text={line.text} kind="words" />
      ) : (
        <ul className="summary-sentences">
          {line.sentences.map((sentence) => (
            <li
              key={sentence.key}
              data-strip={sentence.key}
              data-testid={fullSet.has(sentence.key) ? `summary-${sentence.key}` : undefined}
            >
              <SentenceView parts={sentence.parts} names={names} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

/**
 * The summary strip for the loaded snapshot; `onFull` hears the keys of the sentences it shows in
 * full ('' while it shows none, loads or failed).
 */
export function SummaryStrip({
  snapshot,
  onFull,
}: {
  snapshot: GraphSnapshot;
  onFull: (keys: string) => void;
}): ReactElement {
  const summary = useMapStore((state) => state.summary);
  const noteOutOfStep = useMapStore((state) => state.noteOutOfStep);
  // One array per summary and snapshot, so the strip refits only when they change.
  const text = useMemo(() => summaryText(summary, snapshot), [summary, snapshot]);
  const names = namesOfFiles(snapshot.files);
  const isShown = text?.ok === true;
  const isBehind = isSummaryOutOfStep(summary, text);
  useEffect(() => {
    if (!isShown) onFull('');
  }, [isShown, onFull]);
  useEffect(() => {
    if (isBehind) noteOutOfStep();
  }, [isBehind, noteOutOfStep, snapshot]);
  return (
    <section className="summary" aria-label="Summary">
      {text === null ? (
        <p className="summary-state">Loading the summary.</p>
      ) : text.ok ? (
        <StripSentences sentences={text.sentences} names={names} onFull={onFull} />
      ) : (
        <p className="summary-state" role="alert">
          <ClippedText
            text={`The summary could not be loaded: ${whyNotLoaded(text.error)}.`}
            kind="words"
          />
        </p>
      )}
    </section>
  );
}
