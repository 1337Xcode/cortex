// The camera controls at the bottom left: Fit (Z), Center (C), and the zoom percentage with − and +.
// Native buttons with their shortcut shown beside the name.
import type { ReactElement } from 'react';
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
      <button
        type="button"
        className="pill"
        aria-keyshortcuts="Z"
        onClick={() => {
          onAction('fit');
        }}
      >
        <span>Fit</span>
        <KeyHint name="Z" />
      </button>
      <button
        type="button"
        className="pill"
        aria-keyshortcuts="C"
        onClick={() => {
          onAction('center');
        }}
      >
        <span>Center</span>
        <KeyHint name="C" />
      </button>
      <div className="pill zoom-row">
        <output className="zoom-value" aria-label="Zoom">
          {formatZoom(zoom)}
        </output>
        <button
          type="button"
          className="zoom-step"
          aria-label="Zoom out"
          aria-keyshortcuts="-"
          onClick={() => {
            onAction('zoomOut');
          }}
        >
          −
        </button>
        <button
          type="button"
          className="zoom-step"
          aria-label="Zoom in"
          aria-keyshortcuts="+"
          onClick={() => {
            onAction('zoomIn');
          }}
        >
          +
        </button>
      </div>
    </div>
  );
}
