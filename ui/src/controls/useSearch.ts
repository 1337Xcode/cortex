// The search box's state: the text, the daemon's answer to it once the person pauses typing, and
// whether the result list is open. Only the answer to the current text is ever kept; an older
// request is cancelled when the text changes. Each result names its file by id and by path, and
// counts only where the snapshot on screen holds that path: a result whose file the snapshot does
// not hold yet is listed by its path, never opened, and the map is asked to read its data again.
// Enter picks the first result the map holds, now or as soon as the answer arrives.
import { useEffect, useRef, useState } from 'react';
import type { GraphFile } from '../api/graph';
import type { ApiError } from '../api/guard';
import { fetchSearch, isValidQuery, type SearchResult } from '../api/search';
import { isOutOfStep, nameFile, type NamedFile } from '../map/namedFile';
import type { Scene } from '../map/scene';
import { mapStore } from '../state/store';
import { SEARCH_DEBOUNCE_MS } from '../theme/tokens';

/** What the search box knows about its text. */
export type SearchState =
  | { readonly status: 'idle' }
  | { readonly status: 'too_long' }
  | { readonly status: 'loading'; readonly query: string }
  | { readonly status: 'error'; readonly query: string; readonly error: ApiError }
  | { readonly status: 'ready'; readonly query: string; readonly results: readonly SearchResult[] };

/** A result and what the snapshot on screen makes of its file. */
export interface FoundResult {
  readonly result: SearchResult;
  readonly named: NamedFile;
}

/** The state a new text starts in, before any answer. */
export function stateForText(text: string): SearchState {
  if (text.trim() === '') return { status: 'idle' };
  return isValidQuery(text) ? { status: 'loading', query: text } : { status: 'too_long' };
}

/** Each of `results` with its file found in `files` by its path (see `nameFile`). */
export function foundResults(
  files: readonly Pick<GraphFile, 'path'>[],
  results: readonly SearchResult[],
): FoundResult[] {
  return results.map((result) => ({ result, named: nameFile(files, result.file, result.path) }));
}

/**
 * Whether any of `results` names its file by an id `files` holds another file at, or none: the
 * snapshot on screen is older than the daemon's ids.
 */
export function isAnswerAhead(
  files: readonly Pick<GraphFile, 'path'>[],
  results: readonly SearchResult[],
): boolean {
  return results.some((result) => isOutOfStep(files, result.file, result.path));
}

/** The id of the first result's file the snapshot holds, or null when it holds none of them. */
export function firstListedId(found: readonly FoundResult[]): number | null {
  for (const { named } of found) if (named.kind === 'listed') return named.id;
  return null;
}

/** The search box's state and what its keys and buttons do. */
export interface Search {
  readonly text: string;
  readonly state: SearchState;
  /** The current answer's results against the snapshot on screen; empty without an answer. */
  readonly found: readonly FoundResult[];
  readonly isListOpen: boolean;
  readonly setText: (text: string) => void;
  /** Picks the first result the map holds, or the first to arrive for the current text. */
  readonly submit: () => void;
  /** Selects the file of `id`, flies to it and closes the list. */
  readonly choose: (id: number) => void;
  /** Closes the list; returns whether it was open. */
  readonly close: () => boolean;
}

/** Runs the search box for the map of `scene`. */
export function useSearch(scene: Scene): Search {
  const [text, setTextState] = useState('');
  const [state, setState] = useState<SearchState>({ status: 'idle' });
  const [isListOpen, setIsListOpen] = useState(false);
  const pickWhenReady = useRef(false);
  const { files } = scene.snapshot;
  const found = state.status === 'ready' ? foundResults(files, state.results) : [];

  const choose = (id: number): void => {
    pickWhenReady.current = false;
    setIsListOpen(false);
    mapStore.getState().selectFile(id, { fly: true });
  };

  useEffect(() => {
    if (state.status !== 'loading') return undefined;
    const query = state.query;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void fetchSearch(query, controller.signal).then((result) => {
        if (controller.signal.aborted) return;
        if (!result.ok) {
          setState({ status: 'error', query, error: result.error });
          return;
        }
        const { results } = result.value;
        setState({ status: 'ready', query, results });
        if (isAnswerAhead(scene.snapshot.files, results)) mapStore.getState().noteOutOfStep();
        const first = firstListedId(foundResults(scene.snapshot.files, results));
        if (pickWhenReady.current && first !== null) {
          pickWhenReady.current = false;
          setIsListOpen(false);
          mapStore.getState().selectFile(first, { fly: true });
        }
      });
    }, SEARCH_DEBOUNCE_MS);
    return () => {
      window.clearTimeout(timer);
      controller.abort();
    };
  }, [state, scene]);

  return {
    text,
    state,
    found,
    isListOpen,
    setText: (next) => {
      pickWhenReady.current = false;
      setTextState(next);
      setState(stateForText(next));
      setIsListOpen(next.trim() !== '');
    },
    submit: () => {
      const first = firstListedId(found);
      if (first !== null) choose(first);
      else if (state.status === 'loading') pickWhenReady.current = true;
    },
    choose,
    close: () => {
      pickWhenReady.current = false;
      setIsListOpen(false);
      return isListOpen;
    },
  };
}
