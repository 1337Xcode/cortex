// A plain-text line that says which file is selected and how many linked files stay lit, for
// screen readers and for anyone who cannot see the dimming.
import type { ReactElement } from 'react';
import { countOf } from '../text/format';
import { neighbourhoodOf } from './highlight';
import type { Scene } from './scene';

/** Describes the focus in one sentence, or says nothing is highlighted. */
function describeFocus(scene: Scene, focus: number | null, minStrength: number): string {
  const neighbourhood = neighbourhoodOf(scene, focus, minStrength);
  if (neighbourhood === null) return 'No file is highlighted.';
  const path = scene.snapshot.files[neighbourhood.focus]?.path ?? '';
  const linked = countOf(neighbourhood.neighbours.length, 'linked file');
  return `Highlighting ${path} and ${linked}; other files are dimmed.`;
}

/**
 * The status line, announced politely when the selection changes. Hover is not announced: moving
 * the pointer across the map would read out every file it passes.
 */
export function MapStatus(props: {
  scene: Scene;
  selected: number | null;
  minStrength: number;
}): ReactElement {
  return (
    <p className="visually-hidden" role="status" aria-live="polite" data-testid="map-status">
      {describeFocus(props.scene, props.selected, props.minStrength)}
    </p>
  );
}
