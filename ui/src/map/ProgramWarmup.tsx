// A hidden deck.gl canvas shown only while the snapshot loads. Its layers (warmup.ts) make
// deck.gl compile the map's shaders and start every program's link at once, so the GPU driver
// compiles them side by side while the page waits for the daemon. The browser keeps the linked
// programs, and the map's own canvas, created next with the same settings, links them again from
// that cache at almost no cost.
import DeckGL from '@deck.gl/react';
import { useMemo, type ReactElement } from 'react';
import { useThemeName } from '../theme/media';
import { THEMES } from '../theme/tokens';
import { MAP_VIEW } from './layers';
import { warmupLayers } from './warmup';

/** Prepares the map's GPU programs out of sight; unmount it once the map itself is shown. */
export function ProgramWarmup(): ReactElement {
  const colors = THEMES[useThemeName()];
  const layers = useMemo(() => warmupLayers(colors), [colors]);
  return (
    <div className="visually-hidden" aria-hidden="true">
      <DeckGL views={MAP_VIEW} layers={layers} />
    </div>
  );
}
