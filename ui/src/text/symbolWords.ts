// The words of a file's symbols as the panel lists them: what each is and where, how many of the
// latest work orders that listed the file selected lines of it, and what the section holds.
import {
  MAX_TOLD,
  type FileDetail,
  type FileSymbol,
  type SelectedSpan,
  type SymbolKind,
} from '../api/file';
import { countOf, formatCount } from './format';

/** "lines 40 to 88", or "line 40" for a span of one line. */
export function linesText(span: {
  readonly start_line: number;
  readonly end_line: number;
}): string {
  return span.start_line === span.end_line
    ? `line ${formatCount(span.start_line)}`
    : `lines ${formatCount(span.start_line)} to ${formatCount(span.end_line)}`;
}

/** "selected by 2 work orders", or "" for a symbol none selected. */
export function selectedText(orders: number): string {
  return orders === 0 ? '' : `selected by ${countOf(orders, 'work order')}`;
}

/** Each kind of symbol with its article, as a sentence names it: "a class", "an interface". */
export const SYMBOL_KIND_PHRASES: Readonly<Record<SymbolKind, string>> = {
  function: 'a function',
  method: 'a method',
  class: 'a class',
  interface: 'an interface',
  type: 'a type',
  constant: 'a constant',
  module: 'a module',
  test: 'a test',
};

/** One symbol in a sentence: "OrderBook, a class at lines 40 to 120, selected by 2 work orders." */
export function symbolSentence(symbol: FileSymbol): string {
  const selected = selectedText(symbol.selected);
  const nested = symbol.is_nested ? ' inside another symbol' : '';
  return `${symbol.name}, ${SYMBOL_KIND_PHRASES[symbol.kind]}${nested} at ${linesText(symbol)}${selected === '' ? '' : `, ${selected}`}.`;
}

/** One selected span in a sentence: "Lines 40 to 88, selected by 2 work orders." */
export function spanSentence(span: SelectedSpan): string {
  const lines = linesText(span);
  return `${lines.charAt(0).toUpperCase()}${lines.slice(1)}, ${selectedText(span.orders)}.`;
}

/** What the section says above its lists: how many symbols, and what the highlight means. */
export function symbolsNote(
  detail: Pick<FileDetail, 'symbols' | 'symbols_total' | 'selected'>,
): string {
  const listed =
    detail.symbols.length < detail.symbols_total
      ? `The first ${formatCount(detail.symbols.length)} of ${countOf(detail.symbols_total, 'symbol')}, in line order`
      : `${countOf(detail.symbols_total, 'symbol')}, in line order`;
  const chosen = detail.symbols.filter((symbol) => symbol.selected > 0).length;
  const orders = `Cortex's last ${formatCount(MAX_TOLD)} work orders that listed this file`;
  const highlight =
    detail.selected.length === 0
      ? `${orders} picked no lines of it.`
      : `${orders} picked lines inside ${countOf(chosen, 'symbol')}, highlighted below.`;
  return `${listed}, as Cortex last indexed the file. ${highlight}`;
}

/** What the section says when a symbol cannot open at its own line in the person's editor. */
export const AT_FIRST_LINE_NOTE =
  'Each symbol opens the file at line 1: the editor address in ui.editor_url does not show where its line number goes.';
