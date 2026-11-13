// The search box at the top: a path, a file name or a symbol. `/` focuses it from anywhere; Enter
// flies to the first result the map holds and selects it; the arrow keys walk the results; Escape
// closes them. A result whose file the map does not hold yet is a row of text with its whole path
// and "not on the map yet", never a button that would open another file.
import { useId, useRef, type KeyboardEvent, type ReactElement, type RefObject } from 'react';
import { visibleText } from '../api/guard';
import { ClippedLabel } from '../components/Clipped';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import type { SearchResult } from '../api/search';
import { namedPath } from '../map/namedFile';
import type { Scene } from '../map/scene';
import { namedFileNote, namedFileSentence } from '../text/namedFileWords';
import { useSearch, type FoundResult, type SearchState } from './useSearch';

function stateLine(state: SearchState): string | null {
  switch (state.status) {
    case 'idle':
      return null;
    case 'too_long':
      return 'Searches are at most 100 characters.';
    case 'loading':
      return 'Searching.';
    case 'error':
      return `Search failed (${state.error.code}): ${state.error.message}. Try again in a moment.`;
    case 'ready':
      return state.results.length === 0
        ? `Nothing matches "${state.query}". Search by part of a path, a file name or a symbol.`
        : null;
  }
}

/** What a result is, as its row names it. */
function kindWord(result: SearchResult): string {
  return result.kind === 'symbol' ? 'symbol' : 'file';
}

/**
 * One result: a button that flies to its file when the map holds it, else a row of text naming
 * the file by its whole path and saying it is not on the map yet.
 */
function ResultRow(props: {
  found: FoundResult;
  onChoose: (id: number) => void;
  onKey: (event: KeyboardEvent<HTMLButtonElement>) => void;
}): ReactElement {
  const { result, named } = props.found;
  const path = namedPath(named) ?? result.path;
  const where = result.line === null ? path : `${path}:${result.line}`;
  const note = namedFileNote(named);
  const name = visibleText(result.name);
  const body = (
    <>
      <IconSlot icon={fileIcon(path)} />
      <ClippedLabel text={name} kind="words" className="search-result-name" />
      <span className="search-result-kind">
        {note === null ? kindWord(result) : `${kindWord(result)}, ${note}`}
      </span>
      <ClippedLabel text={where} kind="path" className="search-result-path" />
    </>
  );
  if (named.kind !== 'listed') {
    return (
      <p className="search-result" data-listed="false">
        <span aria-hidden="true" className="contents">
          {body}
        </span>
        <span className="visually-hidden">
          {`${name}, ${kindWord(result)}, ${where}. ${namedFileSentence(named) ?? ''}`}
        </span>
      </p>
    );
  }
  const id = named.id;
  return (
    <button
      type="button"
      className="search-result"
      aria-label={`${name}, ${kindWord(result)}, ${where}`}
      onClick={() => {
        props.onChoose(id);
      }}
      onKeyDown={props.onKey}
    >
      {body}
    </button>
  );
}

/** A result's key in the list: what it is, where, and its name. */
function resultKey(result: SearchResult): string {
  return `${result.kind}:${result.path}:${result.name}:${result.line ?? 0}`;
}

/** The index of `found`'s button among the results' buttons (the rows the map holds). */
function buttonIndex(results: readonly FoundResult[], found: FoundResult): number {
  return results.filter((entry) => entry.named.kind === 'listed').indexOf(found);
}

/** Moves focus among the result buttons, or back to the input above the first. */
function moveFocus(
  list: HTMLElement | null,
  input: HTMLInputElement | null,
  from: number,
  step: 1 | -1,
): void {
  const buttons = list?.querySelectorAll('button') ?? [];
  const target = from + step;
  if (target < 0) {
    input?.focus();
    return;
  }
  const next = buttons[Math.min(target, buttons.length - 1)];
  if (next instanceof HTMLButtonElement) next.focus();
}

/** The search box and its result list. */
export function SearchBox({
  scene,
  inputRef,
}: {
  scene: Scene;
  inputRef: RefObject<HTMLInputElement | null>;
}): ReactElement {
  const search = useSearch(scene);
  const listRef = useRef<HTMLUListElement>(null);
  const inputId = useId();
  const listId = useId();
  const line = stateLine(search.state);
  const results = search.found;
  const isShown = search.isListOpen && (results.length > 0 || line !== null);

  const onInputKey = (event: KeyboardEvent<HTMLInputElement>): void => {
    if (event.key === 'Enter') {
      event.preventDefault();
      search.submit();
    } else if (event.key === 'ArrowDown' && results.length > 0) {
      event.preventDefault();
      moveFocus(listRef.current, inputRef.current, -1, 1);
    } else if (event.key === 'Escape' && search.close()) {
      event.preventDefault();
    }
  };
  const onResultKey =
    (index: number) =>
    (event: KeyboardEvent<HTMLButtonElement>): void => {
      if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp' && event.key !== 'Escape') return;
      event.preventDefault();
      if (event.key === 'Escape') {
        search.close();
        inputRef.current?.focus();
      } else {
        moveFocus(listRef.current, inputRef.current, index, event.key === 'ArrowDown' ? 1 : -1);
      }
    };

  return (
    <form
      className="search"
      role="search"
      onSubmit={(event) => {
        event.preventDefault();
      }}
    >
      <label htmlFor={inputId} className="visually-hidden">
        Search files and symbols
      </label>
      <input
        id={inputId}
        ref={inputRef}
        className="search-input"
        type="search"
        autoComplete="off"
        spellCheck={false}
        placeholder="Search files and symbols"
        aria-keyshortcuts="/"
        aria-controls={listId}
        value={search.text}
        onChange={(event) => {
          search.setText(event.target.value);
        }}
        onKeyDown={onInputKey}
      />
      <kbd className="key-hint search-key" aria-hidden="true">
        /
      </kbd>
      <div
        className="search-popover"
        id={listId}
        hidden={!isShown}
        data-overlay="true"
        data-scroll-area="true"
      >
        {line === null ? null : (
          <p className="search-state" role="status">
            {line}
          </p>
        )}
        <ul className="search-results" ref={listRef} aria-label="Search results">
          {results.map((found) => (
            <li key={resultKey(found.result)}>
              <ResultRow
                found={found}
                onChoose={search.choose}
                onKey={onResultKey(buttonIndex(results, found))}
              />
            </li>
          ))}
        </ul>
      </div>
    </form>
  );
}
