// Which summary sentences the one-line strip shows: in order of use, each in full or in its short
// form, as many as fit whole. Nothing is cut mid-word: a later sentence that does not fit is left
// to the overview panel, which always shows them all. When not even the first sentence fits in its
// short form, the strip shows that short form cut at a word boundary (whole on hover and focus).
import type { SentencePart, SummarySentence } from './summarySentences';

/** A sentence the strip shows, in the form that fitted. */
export interface StripSentence {
  readonly key: SummarySentence['key'];
  readonly parts: readonly SentencePart[];
}

/** The strip's line: whole sentences, or the first one's short form to cut to the width. */
export type StripLine =
  | { readonly kind: 'sentences'; readonly sentences: readonly StripSentence[] }
  | { readonly kind: 'cut'; readonly text: string };

/** How the strip reads a sentence: its text as shown, and that text's width in pixels. */
export interface StripText {
  readonly textOf: (parts: readonly SentencePart[]) => string;
  readonly measure: (text: string) => number;
}

/** The sentences that fit `widthPx`, `gapPx` apart. */
export function fitStrip(
  sentences: readonly SummarySentence[],
  widthPx: number,
  gapPx: number,
  text: StripText,
): StripLine {
  const widthOf = (parts: readonly SentencePart[]): number => text.measure(text.textOf(parts));
  const shown: StripSentence[] = [];
  let remaining = widthPx;
  for (const [index, sentence] of sentences.entries()) {
    const gap = shown.length === 0 ? 0 : gapPx;
    const form = [sentence.parts, sentence.short].find(
      (parts) => widthOf(parts) + gap <= remaining,
    );
    if (form === undefined && index === 0) {
      return { kind: 'cut', text: text.textOf(sentence.short) };
    }
    if (form === undefined) continue;
    shown.push({ key: sentence.key, parts: form });
    remaining -= widthOf(form) + gap;
  }
  return { kind: 'sentences', sentences: shown };
}
