// The facts anchored to the selected file, with the person's say on each: a candidate gets
// "Confirm", an active fact "Retire", a suspect fact (its code changed since) "Confirm again" and
// "Retire". Each is a native button. Retiring asks first, in the row, with "Keep it" focused;
// Escape there keeps the fact and gives the focus back. While the daemon writes the word, the
// facts' buttons wait and the one pressed says so; then what came of it is said in words for
// everyone, screen readers included, and the map reads its numbers again, this panel with them.
import { useEffect, useRef, useState, type ReactElement } from 'react';
import type { FileDetail, FileFact } from '../api/file';
import { mayHaveWritten, sendFactWord, type FactWord } from '../api/factWord';
import { visibleText } from '../api/guard';
import { Button } from '../components/ui/button';
import { PAGE_REFRESHERS } from '../state/pageRefreshers';
import { useMapStore, type DetailState } from '../state/store';
import {
  outcomeSentence,
  RETIRE_CHOICES,
  retireQuestion,
  STATUS_NAMES,
  WAITING_LABEL,
  wordLabel,
  wordsFor,
} from '../text/factWords';
import { DetailSection } from './fileSections';

/** Where the focus goes once the section has drawn its state: what came of a word, or a retire button. */
type FocusTarget = { readonly to: 'said' } | { readonly to: 'retire'; readonly id: number } | null;

/** What the section is doing with the person's words. */
interface WordState {
  /** The fact whose retire step is open, or null. */
  readonly asking: number | null;
  /** The word on its way to the daemon, or null. */
  readonly sending: { readonly id: number; readonly word: FactWord } | null;
  /** What came of the last word, said in full, or null. */
  readonly said: string | null;
  /** Where the focus goes once this state is drawn. */
  readonly focus: FocusTarget;
}

const NOTHING: WordState = { asking: null, sending: null, said: null, focus: null };

/** What a fact's row may do with the person's words. */
interface WordActions {
  readonly state: WordState;
  readonly send: (id: number, word: FactWord) => void;
  readonly ask: (id: number) => void;
  readonly keep: (id: number) => void;
}

/** The retire step of fact `id`: the question, "Retire it" and "Keep it" (focused). */
function RetireStep({ id, actions }: { id: number; actions: WordActions }): ReactElement {
  const keepRef = useRef<HTMLButtonElement>(null);
  useEffect(() => {
    keepRef.current?.focus();
  }, []);
  return (
    <div
      className="fact-ask"
      role="group"
      aria-label={`Retire fact ${id}`}
      onKeyDown={(event) => {
        if (event.key !== 'Escape') return;
        // The panel closes on Escape; here it only closes the question.
        event.preventDefault();
        event.stopPropagation();
        actions.keep(id);
      }}
    >
      <p className="fact-ask-text">{retireQuestion(id)}</p>
      <div className="fact-words">
        <Button
          size="compact"
          onClick={() => {
            actions.send(id, 'retire');
          }}
        >
          {RETIRE_CHOICES.yes}
        </Button>
        <Button
          size="compact"
          ref={keepRef}
          onClick={() => {
            actions.keep(id);
          }}
        >
          {RETIRE_CHOICES.no}
        </Button>
      </div>
    </div>
  );
}

/** One fact: its status, kind and text, and the words its status offers. */
function FactRow({ fact, actions }: { fact: FileFact; actions: WordActions }): ReactElement {
  const { sending, asking, focus } = actions.state;
  const isMine = sending?.id === fact.id;
  const retireRef = useRef<HTMLButtonElement>(null);
  const takesFocus = focus?.to === 'retire' && focus.id === fact.id && asking !== fact.id;
  useEffect(() => {
    // The retire button comes back once the question closes; it takes the focus back.
    if (takesFocus) retireRef.current?.focus();
  }, [takesFocus]);
  return (
    <li data-status={fact.status} data-testid={`fact-${fact.id}`} aria-busy={isMine}>
      <span className="fact-status">{STATUS_NAMES[fact.status]}</span>
      <span className="fact-kind">{visibleText(fact.kind)}</span>
      <p className="fact-text">{visibleText(fact.text)}</p>
      {asking === fact.id ? (
        <RetireStep id={fact.id} actions={actions} />
      ) : (
        <div className="fact-words">
          {wordsFor(fact.status).map((word) => (
            <Button
              key={word}
              ref={word === 'retire' ? retireRef : undefined}
              size="compact"
              data-fact-word={word}
              disabled={sending !== null}
              aria-label={`${wordLabel(word, fact.status)} fact ${fact.id}`}
              onClick={() => {
                if (word === 'retire') actions.ask(fact.id);
                else actions.send(fact.id, word);
              }}
            >
              {isMine && sending.word === word ? WAITING_LABEL[word] : wordLabel(word, fact.status)}
            </Button>
          ))}
        </div>
      )}
    </li>
  );
}

/** The facts, each with its words. */
function FactList({ detail, actions }: { detail: FileDetail; actions: WordActions }): ReactElement {
  return (
    <ul className="fact-list">
      {detail.facts.map((fact) => (
        <FactRow key={fact.id} fact={fact} actions={actions} />
      ))}
    </ul>
  );
}

/**
 * The facts anchored to the file; suspect ones are marked in words and colour. Open at first. The
 * panel keys it by the file, so what came of a word is said only in its own file's panel.
 */
export function FactsSection({ detail, id }: { detail: DetailState; id: number }): ReactElement {
  const refresh = useMapStore((store) => store.refresh);
  const [state, setState] = useState<WordState>(NOTHING);
  const saidRef = useRef<HTMLParagraphElement>(null);
  const isMounted = useRef(true);
  useEffect(() => {
    isMounted.current = true;
    return () => {
      isMounted.current = false;
    };
  }, []);
  useEffect(() => {
    // The button pressed may be gone with its fact; the focus waits on what came of it.
    if (state.focus?.to === 'said') saidRef.current?.focus();
  }, [state]);
  const actions: WordActions = {
    state,
    send: (fact, word) => {
      setState({ asking: null, sending: { id: fact, word }, said: null, focus: null });
      void sendFactWord(fact, word).then(async (outcome) => {
        // The panel may have closed while the word was on its way: nothing is said, but the map
        // is still read again below.
        if (isMounted.current) {
          const said = outcomeSentence(fact, word, outcome);
          setState({ asking: null, sending: null, said, focus: { to: 'said' } });
        }
        // A word that was written, or may have been (taken too late to say, or answered in a way
        // the page cannot read, or not at all), changes this panel and the map's numbers; a
        // refusal and a failed write changed nothing.
        if (mayHaveWritten(outcome)) await refresh(PAGE_REFRESHERS);
      });
    },
    ask: (fact) => {
      setState({ ...state, asking: fact, said: null, focus: null });
    },
    keep: (fact) => {
      setState({ ...state, asking: null, focus: { to: 'retire', id: fact } });
    },
  };
  return (
    <DetailSection
      title="Facts"
      testId="panel-facts"
      detail={detail}
      id={id}
      isOpenAtFirst
      countOf={(d) => d.facts.length}
      empty="No facts are recorded for this file."
      footer={
        <p className="fact-said" role="status" data-testid="fact-said" tabIndex={-1} ref={saidRef}>
          {state.said ?? ''}
        </p>
      }
    >
      {(d) => <FactList detail={d} actions={actions} />}
    </DetailSection>
  );
}
