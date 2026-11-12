// The line that says the live view is paused: its connection to Cortex was lost, the markers are
// hidden rather than shown stale, and the page is trying again. It sits with the map's controls,
// so it is seen at every screen size, and is announced once.
import type { ReactElement } from 'react';

/** Says the live view is paused and reconnecting. */
export function LivePaused(): ReactElement {
  return (
    <p className="live-paused" role="status" data-testid="live-paused">
      Live view paused, reconnecting.
    </p>
  );
}
