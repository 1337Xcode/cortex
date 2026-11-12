// The switch between the map's views, beside the camera controls: one native button per view,
// pressed for the active one, each saying the question its view answers; keys 1 to 5 do the same,
// shown once at the end of the switch.
// Switching changes only colours: the camera and the layout stay where they are.
import { useEffect, type ReactElement } from 'react';
import { Button } from '../components/ui/button';
import { useMapStore } from '../state/store';
import { VIEWS } from '../views/registry';
import { isKeyField, viewIndexFor } from './shortcuts';

/** Picks a view when its digit is pressed outside a text field. */
function useViewKeys(setView: (name: string) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      const { key, ctrlKey, altKey, metaKey } = event;
      const isInField = isKeyField(event.target);
      const index = viewIndexFor({ key, ctrlKey, altKey, metaKey, isInField }, VIEWS.length);
      const view = index === null ? undefined : VIEWS[index];
      if (view === undefined) return;
      event.preventDefault();
      setView(view.name);
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, [setView]);
}

/** The view switch. */
export function ViewSwitch(): ReactElement {
  const active = useMapStore((state) => state.view);
  const setView = useMapStore((state) => state.setView);
  useViewKeys(setView);
  return (
    <div className="view-switch" role="group" aria-label="View">
      {VIEWS.map((view, index) => {
        const key = String(index + 1);
        return (
          <Button
            key={view.name}
            variant="ghost"
            size="compact"
            className="view-option"
            aria-pressed={view.name === active}
            aria-keyshortcuts={key}
            title={`${view.title}: ${view.question} (${key})`}
            onClick={() => {
              setView(view.name);
            }}
          >
            {view.title}
          </Button>
        );
      })}
      <kbd className="key-hint" aria-hidden="true">
        {`1–${VIEWS.length}`}
      </kbd>
    </div>
  );
}
