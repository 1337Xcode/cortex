// The one sentence that says what the picture is, shown on first open. Dismissing it is remembered
// in the browser's local storage, so it shows once; the legend opens it again. Where the browser
// refuses local storage, the sentence shows on each open and the refusal is logged as a warning.
import { useEffect, type ReactElement } from 'react';
import { Button } from '../components/ui/button';
import { useMapStore } from '../state/store';
import { INTRO_SENTENCE } from '../text/vocabulary';

/** Where the dismissal is remembered. */
export const INTRO_STORAGE_KEY = 'cortex-map-intro-dismissed';

/** Whether the person dismissed the sentence before; false where storage cannot be read. */
function wasDismissed(): boolean {
  try {
    return window.localStorage.getItem(INTRO_STORAGE_KEY) === 'true';
  } catch (error) {
    console.warn('the map cannot read local storage, so the first-open sentence shows', error);
    return false;
  }
}

/** Remembers the dismissal; where storage cannot be written, the sentence shows again next time. */
function rememberDismissed(): void {
  try {
    window.localStorage.setItem(INTRO_STORAGE_KEY, 'true');
  } catch (error) {
    console.warn(
      'the map cannot write local storage, so the first-open sentence shows again',
      error,
    );
  }
}

/** Opens the sentence on mount unless it was dismissed before. */
export function useIntroOnFirstOpen(): void {
  const setIntroOpen = useMapStore((state) => state.setIntroOpen);
  useEffect(() => {
    if (!wasDismissed()) setIntroOpen(true);
  }, [setIntroOpen]);
}

/** The sentence and its dismiss button, while open. */
export function Intro(): ReactElement | null {
  const isOpen = useMapStore((state) => state.isIntroOpen);
  const setIntroOpen = useMapStore((state) => state.setIntroOpen);
  if (!isOpen) return null;
  return (
    <section className="intro" aria-label="What this map shows">
      <p className="intro-text">{INTRO_SENTENCE}</p>
      <Button
        size="compact"
        className="intro-dismiss"
        onClick={() => {
          rememberDismissed();
          setIntroOpen(false);
        }}
      >
        Got it
      </Button>
    </section>
  );
}
