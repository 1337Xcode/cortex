// The map's camera as React state: fitted to the graph until the person moves it, moved by the
// pointer (through deck.gl), the controls and the keyboard, and flown to a file when the store asks.
// Moves the page makes are animated for 150 ms, or not at all under reduced motion.
import { LinearInterpolator, type OrthographicViewState } from '@deck.gl/core';
import { useCallback, useEffect, useMemo, useState } from 'react';
import { isKeyField, shortcutFor, type ShortcutAction } from '../controls/shortcuts';
import { useMapStore, type FlyRequest } from '../state/store';
import { useReducedMotion } from '../theme/media';
import { CAMERA, MOTION } from '../theme/tokens';
import {
  cameraOnFile,
  cameraOnHub,
  fitCamera,
  lookAt,
  panBy,
  stepZoom,
  type Camera,
  type Insets,
} from './camera';
import type { ScreenSize } from './labels';
import type { Scene } from './scene';

const INTERPOLATOR = new LinearInterpolator({ transitionProps: ['target', 'zoom'] });

/** The camera a view state shows. */
function cameraOf(viewState: OrthographicViewState): Camera {
  const zoom = viewState.zoom ?? 0;
  return {
    x: viewState.target?.[0] ?? 0,
    y: viewState.target?.[1] ?? 0,
    zoom: typeof zoom === 'number' ? zoom : zoom[0],
  };
}

/** The view state for a camera, animated over `durationMs` from wherever the view is now. */
function viewStateOf(camera: Camera, durationMs: number): OrthographicViewState {
  return {
    target: [camera.x, camera.y, 0],
    zoom: camera.zoom,
    minZoom: CAMERA.minZoom,
    maxZoom: CAMERA.maxZoom,
    transitionDuration: durationMs,
    transitionInterpolator: INTERPOLATOR,
  };
}

/** What a camera move needs to know besides the camera. */
export interface CameraContext {
  readonly scene: Scene;
  readonly screen: ScreenSize;
  readonly insets: Insets;
  readonly selected: number | null;
}

/** Where a camera action leaves the camera; null for actions the map does not own. */
function moveFor(action: ShortcutAction, camera: Camera, context: CameraContext): Camera | null {
  const { scene, screen, insets, selected } = context;
  const step = CAMERA.panStepPx;
  switch (action) {
    case 'fit':
      return fitCamera(scene, screen, insets);
    case 'center': {
      if (selected === null) {
        const { minX, minY, maxX, maxY } = scene.bounds;
        return lookAt((minX + maxX) / 2, (minY + maxY) / 2, camera.zoom, insets);
      }
      const { fileX, fileY } = scene.layout;
      return lookAt(fileX[selected] ?? 0, fileY[selected] ?? 0, camera.zoom, insets);
    }
    case 'zoomIn':
      return stepZoom(camera, 1);
    case 'zoomOut':
      return stepZoom(camera, -1);
    case 'panLeft':
      return panBy(camera, -step, 0);
    case 'panRight':
      return panBy(camera, step, 0);
    case 'panUp':
      return panBy(camera, 0, -step);
    case 'panDown':
      return panBy(camera, 0, step);
    case 'focusSearch':
    case 'toggleLegend':
    case 'clear':
      return null;
  }
}

/** Where a fly request takes the camera; null for a folder the layout does not have. */
function flyTarget(
  scene: Scene,
  camera: Camera,
  flyTo: FlyRequest,
  view: { readonly screen: ScreenSize; readonly insets: Insets },
): Camera | null {
  if (flyTo.target === 'file') return cameraOnFile(scene, camera, flyTo.file, view.insets);
  const hub = scene.layout.hubPath.indexOf(flyTo.folder);
  return hub < 0 ? null : cameraOnHub(scene, hub, view.screen, view.insets);
}

/** The camera's state and the ways to move it. */
export interface MapCamera {
  readonly viewState: OrthographicViewState;
  readonly camera: Camera;
  /** Takes a view state deck.gl reports after the pointer moved the view. */
  readonly follow: (viewState: OrthographicViewState) => void;
  /** Moves the camera, animated. */
  readonly moveTo: (camera: Camera) => void;
  /** Runs a camera action from the controls. */
  readonly act: (action: ShortcutAction) => void;
}

/** Runs the camera moves of the keyboard shortcuts while mounted. */
function useCameraKeys(camera: Camera, context: CameraContext, move: (next: Camera) => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      // A control nearer the key (the search results' arrows) has used it already.
      if (event.defaultPrevented) return;
      const { key, ctrlKey, altKey, metaKey } = event;
      const isInField = isKeyField(event.target);
      const action = shortcutFor({ key, ctrlKey, altKey, metaKey, isInField });
      const next = action === null ? null : moveFor(action, camera, context);
      if (next === null) return;
      event.preventDefault();
      move(next);
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, [camera, context, move]);
}

/** Owns the camera of the map of `scene` on a screen of `screen` with chrome over `insets`. */
export function useMapCamera(scene: Scene, screen: ScreenSize, insets: Insets): MapCamera {
  const durationMs = useReducedMotion() ? MOTION.reducedDurationMs : MOTION.durationMs;
  const selected = useMapStore((state) => state.selectedFile);
  const flyTo = useMapStore((state) => state.flyTo);
  const [moved, setMoved] = useState<OrthographicViewState | null>(null);
  const [flownSerial, setFlownSerial] = useState(flyTo?.serial ?? 0);
  const context = useMemo(
    () => ({ scene, screen, insets, selected }),
    [scene, screen, insets, selected],
  );
  const moveTo = useCallback(
    (next: Camera) => {
      setMoved(viewStateOf(next, durationMs));
    },
    [durationMs],
  );

  const viewState = moved ?? viewStateOf(fitCamera(scene, screen, insets), 0);
  const camera = cameraOf(viewState);
  if (flyTo !== null && flyTo.serial !== flownSerial) {
    setFlownSerial(flyTo.serial);
    const next = flyTarget(scene, camera, flyTo, { screen, insets });
    if (next !== null) setMoved(viewStateOf(next, durationMs));
  }
  useCameraKeys(camera, context, moveTo);

  return {
    viewState,
    camera,
    follow: setMoved,
    moveTo,
    act: (action) => {
      const next = moveFor(action, camera, context);
      if (next !== null) moveTo(next);
    },
  };
}
