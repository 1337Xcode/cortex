// The file zoom: the symbols the selected file defines, in line order, those holding lines that the
// latest work orders listing the file selected highlighted with how many orders chose them, and the
// selected spans themselves. With an editor link, each symbol is a link that opens the file at the
// symbol's line, or at line 1 when the editor address does not show where its line goes, which the
// section then says. Closed at first.
import type { ReactElement } from 'react';
import type { FileDetail, FileSymbol } from '../api/file';
import { visibleText } from '../api/guard';
import { ClippedText } from '../components/Clipped';
import { formatCount } from '../text/format';
import {
  AT_FIRST_LINE_NOTE,
  linesText,
  selectedText,
  spanSentence,
  symbolSentence,
  symbolsNote,
} from '../text/symbolWords';
import type { DetailState } from '../state/store';
import { editorHrefAt } from './editorLink';
import { DetailSection } from './fileSections';
import { RowList } from './RowList';

/** One symbol's row: its name and kind, its lines and how often it was selected. */
function SymbolRow({ symbol, detail }: { symbol: FileSymbol; detail: FileDetail }): ReactElement {
  const link = editorHrefAt(detail.editor_url, detail.path, symbol.start_line);
  const body = (
    <>
      <ClippedText text={visibleText(symbol.name)} kind="words" className="symbol-name" />
      <span className="symbol-meta">
        {symbol.kind}, {linesText(symbol)}
        {symbol.selected > 0 ? `, ${selectedText(symbol.selected)}` : ''}
      </span>
    </>
  );
  const shared = {
    'data-selected': symbol.selected > 0,
    'data-nested': symbol.is_nested,
  };
  return link === null ? (
    <p className="symbol-row" {...shared}>
      {body}
    </p>
  ) : (
    <a
      className="symbol-row"
      {...shared}
      href={link.href}
      rel="noreferrer"
      aria-label={`${visibleText(symbolSentence(symbol))} Opens in the editor${link.isAtLine ? ' at its line' : ' at line 1'}.`}
    >
      {body}
    </a>
  );
}

/** The symbols and selected spans, once the detail is loaded. */
function SymbolLists({ detail }: { detail: FileDetail }): ReactElement {
  const opensAtFirst =
    detail.editor_url !== null &&
    detail.symbols[0] !== undefined &&
    editorHrefAt(detail.editor_url, detail.path, detail.symbols[0].start_line)?.isAtLine === false;
  return (
    <>
      <p className="section-note">{symbolsNote(detail)}</p>
      {opensAtFirst ? <p className="section-note">{AT_FIRST_LINE_NOTE}</p> : null}
      {detail.selected.length === 0 ? null : (
        <div className="panel-kind">
          <h4>Lines work orders selected</h4>
          <RowList
            items={detail.selected}
            keyOf={(span) => `${span.start_line}:${span.end_line}`}
            label="Lines work orders selected"
            renderRow={(span) => <p className="span-row">{spanSentence(span)}</p>}
          />
        </div>
      )}
      <div className="panel-kind">
        <h4>Symbols</h4>
        <RowList
          items={detail.symbols.map((symbol, index) => ({ symbol, index }))}
          keyOf={({ index }) => index}
          label="Symbols in line order"
          renderRow={({ symbol }) => <SymbolRow symbol={symbol} detail={detail} />}
        />
      </div>
    </>
  );
}

/** The file's symbols; its count says how many it defines. Closed at first. */
export function SymbolsSection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  return (
    <DetailSection
      title="Symbols"
      testId="panel-symbols"
      detail={detail}
      id={id}
      isOpenAtFirst={false}
      shortcut="symbols"
      countOf={(d) =>
        d.symbols.length < d.symbols_total
          ? `first ${formatCount(d.symbols.length)} of ${formatCount(d.symbols_total)}`
          : d.symbols_total
      }
      empty="Cortex's index holds no symbol for this file: its language is not parsed, or it defines none."
    >
      {(d) => <SymbolLists detail={d} />}
    </DetailSection>
  );
}
