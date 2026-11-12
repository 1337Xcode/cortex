// The camera controls at the bottom of the map: Fit (Z), Center (C), and the zoom percentage with
// − and +. Native buttons with their shortcut shown beside the name; the zoom reads in tabular
// figures so its width does not jump as it changes.
import { Minus, Plus } from 'lucide-react';
import type { ReactElement } from 'react';
import { Button } from '../components/ui/button';
import { formatZoom } from '../map/camera';
import type { ShortcutAction } from './shortcuts';

function KeyHint({ name }: { name: string }): ReactElement {
  return (
    <kbd className="key-hint" aria-hidden="true">
      {name}
    </kbd>
  );
}

/** The camera buttons; each runs `onAction` with its shortcut's action. */
export function Controls({
  zoom,
  onAction,
}: {
  zoom: number;
  onAction: (action: ShortcutAction) => void;
}): ReactElement {
  return (
    <div className="controls" role="group" aria-label="Camera">
      <Button
        aria-keyshortcuts="Z"
        title="Fit the whole map (Z)"
        onClick={() => {
          onAction('fit');
        }}
      >
        <span>Fit</span>
        <KeyHint name="Z" />
      </Button>
      <Button
        aria-keyshortcuts="C"
        title="Center on the selected file (C)"
        onClick={() => {
          onAction('center');
        }}
      >
        <span>Center</span>
        <KeyHint name="C" />
      </Button>
      <div className="zoom-row">
        <Button
          variant="ghost"
          size="icon-compact"
          aria-label="Zoom out"
          aria-keyshortcuts="-"
          title="Zoom out (-)"
          onClick={() => {
            onAction('zoomOut');
          }}
        >
          <Minus aria-hidden="true" />
        </Button>
        <output className="zoom-value" aria-label="Zoom">
          {formatZoom(zoom)}
        </output>
        <Button
          variant="ghost"
          size="icon-compact"
          aria-label="Zoom in"
          aria-keyshortcuts="+"
          title="Zoom in (+)"
          onClick={() => {
            onAction('zoomIn');
          }}
        >
          <Plus aria-hidden="true" />
        </Button>
      </div>
    </div>
  );
}
