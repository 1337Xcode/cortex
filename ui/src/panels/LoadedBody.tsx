// The body of an overview section whose data is asked for only when it opens (the activity, the
// warnings, the daemon's health): it asks once when first shown, says it is loading, says what
// failed with a way to try again, and shows the data once it is there.
import { useEffect, type ReactElement } from 'react';
import { Button } from '../components/ui/button';
import type { PanelState } from '../state/store';
import { whyNotLoaded } from '../text/loadWords';

/** Shows `state`, asking `load` when nothing was asked yet; `what` names the data in sentences. */
export function LoadedBody<T>({
  state,
  load,
  what,
  testId,
  children,
}: {
  state: PanelState<T>;
  load: () => void;
  /** What is loaded, as a sentence names it: "the activity". */
  what: string;
  /** The test id of the failure's sentence. */
  testId: string;
  children: (value: T, receivedMs: number) => ReactElement;
}): ReactElement {
  useEffect(() => {
    if (state.status === 'idle') load();
  }, [state.status, load]);
  switch (state.status) {
    case 'idle':
    case 'loading':
      return <p className="panel-empty">Loading {what}.</p>;
    case 'error':
      return (
        <>
          <p className="panel-empty" role="alert" data-testid={testId}>
            {`${what.charAt(0).toUpperCase()}${what.slice(1)} could not be loaded: ${whyNotLoaded(state.error)}. Check that Cortex is still running (cortex status in a terminal), then try again.`}
          </p>
          <Button
            variant="ghost"
            size="compact"
            className="show-more -ml-sm text-muted-foreground hover:text-foreground"
            onClick={load}
          >
            Try again
          </Button>
        </>
      );
    case 'ready':
      return children(state.value, state.receivedMs);
  }
}
