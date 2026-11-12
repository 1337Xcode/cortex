// The search box at the top: a path, a file name or a symbol. `/` focuses it from anywhere; Enter
// flies to the first result and selects it; the arrow keys walk the results; Escape closes them.
import { useId, useRef, type KeyboardEvent, type ReactElement, type RefObject } from 'react';
import { visibleText } from '../api/guard';
import { ClippedLabel } from '../components/Clipped';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import type { SearchResult } from '../api/search';
import type { Scene } from '../map/scene';
import { useSearch, type SearchState } from './useSearch';

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

function ResultButton(props: {
  result: SearchResult;
  path: string;
  onChoose: (result: SearchResult) => void;
  onKey: (event: KeyboardEvent<HTMLButtonElement>) => void;
}): ReactElement {
  const { result, path } = props;
  const where = result.line === null ? path : `${path}:${result.line}`;
  return (
    <button
      type="button"
      className="search-result"
      aria-label={`${visibleText(result.name)}, ${result.kind === 'symbol' ? 'symbol' : 'file'}, ${where}`}
      onClick={() => {
        props.onChoose(result);
      }}
      onKeyDown={props.onKey}
    >
      <IconSlot icon={fileIcon(path)} />
      <ClippedLabel text={visibleText(result.name)} kind="words" className="search-result-name" />
      <span className="search-result-kind">{result.kind === 'symbol' ? 'symbol' : 'file'}</span>
      <ClippedLabel text={where} kind="path" className="search-result-path" />
    </button>
  );
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
  const results = search.state.status === 'ready' ? search.state.results : [];
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
          {results.map((result, index) => (
            <li key={`${result.kind}:${result.file}:${result.name}:${result.line ?? 0}`}>
              <ResultButton
                result={result}
                path={scene.snapshot.files[result.file]?.path ?? ''}
                onChoose={search.choose}
                onKey={onResultKey(index)}
              />
            </li>
          ))}
        </ul>
      </div>
    </form>
  );
}
