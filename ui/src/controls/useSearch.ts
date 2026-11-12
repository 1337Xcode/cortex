// The search box's state: the text, the daemon's answer to it once the person pauses typing, and
// whether the result list is open. Only the answer to the current text is ever kept; an older
// request is cancelled when the text changes. Results naming a file the map does not have are
// refused with a typed error. Enter picks the first result, now or as soon as the answer arrives.
import { useEffect, useRef, useState } from 'react';
import type { ApiError } from '../api/guard';
import { fetchSearch, isValidQuery, type SearchResult } from '../api/search';
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

/** The state a new text starts in, before any answer. */
export function stateForText(text: string): SearchState {
  if (text.trim() === '') return { status: 'idle' };
  return isValidQuery(text) ? { status: 'loading', query: text } : { status: 'too_long' };
}

/** Checks every result's file against the scene; one unknown id refuses the whole answer. */
export function checkResults(
  scene: Scene,
  query: string,
  results: readonly SearchResult[],
): SearchState {
  const unknown = results.find((result) => result.file >= scene.snapshot.files.length);
  if (unknown === undefined) return { status: 'ready', query, results };
  const message = `a result names file ${unknown.file}, which the map does not have`;
  return { status: 'error', query, error: { code: 'bad_range', message } };
}

/** The search box's state and what its keys and buttons do. */
export interface Search {
  readonly text: string;
  readonly state: SearchState;
  readonly isListOpen: boolean;
  readonly setText: (text: string) => void;
  /** Picks the first result, or the first to arrive for the current text. */
  readonly submit: () => void;
  /** Selects a result's file, flies to it and closes the list. */
  readonly choose: (result: SearchResult) => void;
  /** Closes the list; returns whether it was open. */
  readonly close: () => boolean;
}

/** Runs the search box for the map of `scene`. */
export function useSearch(scene: Scene): Search {
  const [text, setTextState] = useState('');
  const [state, setState] = useState<SearchState>({ status: 'idle' });
  const [isListOpen, setIsListOpen] = useState(false);
  const pickWhenReady = useRef(false);

  const choose = (result: SearchResult): void => {
    pickWhenReady.current = false;
    setIsListOpen(false);
    mapStore.getState().selectFile(result.file, { fly: true });
  };

  useEffect(() => {
    if (state.status !== 'loading') return undefined;
    const query = state.query;
    const controller = new AbortController();
    const timer = window.setTimeout(() => {
      void fetchSearch(query, controller.signal).then((result) => {
        if (controller.signal.aborted) return;
        const next = result.ok
          ? checkResults(scene, query, result.value.results)
          : { status: 'error' as const, query, error: result.error };
        setState(next);
        const first = next.status === 'ready' ? next.results[0] : undefined;
        if (pickWhenReady.current && first !== undefined) {
          pickWhenReady.current = false;
          setIsListOpen(false);
          mapStore.getState().selectFile(first.file, { fly: true });
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
    isListOpen,
    setText: (next) => {
      pickWhenReady.current = false;
      setTextState(next);
      setState(stateForText(next));
      setIsListOpen(next.trim() !== '');
    },
    submit: () => {
      const first = state.status === 'ready' ? state.results[0] : undefined;
      if (first !== undefined) choose(first);
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
