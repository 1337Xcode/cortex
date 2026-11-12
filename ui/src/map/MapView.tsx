// The map: the deck.gl canvas, what pointing at and clicking its nodes does, and the controls and
// legend beside it. Hovering a file highlights it and its neighbours; clicking a file selects it;
// clicking a folder flies in until it opens.
import { OrthographicView, type PickingInfo } from '@deck.gl/core';
import DeckGL from '@deck.gl/react';
import { useMemo, useRef, type ReactElement } from 'react';
import { Controls } from '../controls/Controls';
import { Legend } from '../controls/Legend';
import { useMapStore } from '../state/store';
import { useThemeName } from '../theme/media';
import { THEMES } from '../theme/tokens';
import { cameraOnHub, type Insets } from './camera';
import { buildLayers, LAYER_ID, type FrameBuffers } from './layers';
import { MapStatus } from './MapStatus';
import { createTextMeasurer } from './measureText';
import type { Scene } from './scene';
import { useFrame } from './useFrame';
import { useMapCamera } from './useMapCamera';
import { useElementBox, useScreenSize } from './useScreenSize';

const VIEW = new OrthographicView({ id: 'map', controller: { keyboard: false, inertia: false } });

/** The hub a pick hit, from either hub layer, or null. */
function pickedHub(info: PickingInfo, frame: FrameBuffers): number | null {
  if (info.index < 0) return null;
  const layerId = info.layer?.id;
  const hubs =
    layerId === LAYER_ID.hubs
      ? frame.hubs
      : layerId === LAYER_ID.collapsed
        ? frame.collapsed
        : layerId === LAYER_ID.fileDiscs
          ? frame.fileDiscs
          : null;
  return hubs?.ids[info.index] ?? null;
}

/** The file a pick hit, or null. */
function pickedFile(info: PickingInfo, frame: FrameBuffers): number | null {
  if (info.index < 0 || info.layer?.id !== LAYER_ID.files) return null;
  return frame.files.ids[info.index] ?? null;
}

function cursorFor({
  isDragging,
  isHovering,
}: {
  isDragging: boolean;
  isHovering: boolean;
}): string {
  if (isDragging) return 'grabbing';
  return isHovering ? 'pointer' : 'grab';
}

/** The map of one snapshot. Mount it again (a new `key`) for a new snapshot. */
export function MapView({
  scene,
  receivedMs,
  insets,
}: {
  scene: Scene;
  receivedMs: number;
  /** The canvas edges the page's chrome covers; the camera centres on the rest. */
  insets: Insets;
}): ReactElement {
  const colors = THEMES[useThemeName()];
  const selected = useMapStore((state) => state.selectedFile);
  const hovered = useMapStore((state) => state.hoveredFile);
  const minStrength = useMapStore((state) => state.minStrength);
  const hoverFile = useMapStore((state) => state.hoverFile);
  const selectFile = useMapStore((state) => state.selectFile);
  const containerRef = useRef<HTMLDivElement>(null);
  const screen = useScreenSize(containerRef);
  const controlsRef = useRef<HTMLDivElement>(null);
  const controlsBox = useElementBox(controlsRef);
  const chrome = useMemo(() => [controlsBox], [controlsBox]);
  const measure = useMemo(() => createTextMeasurer(), []);
  const { viewState, camera, follow, moveTo, act } = useMapCamera(scene, screen, insets);
  const frame = useFrame({
    scene,
    camera,
    screen,
    colors,
    selected,
    hovered,
    minStrength,
    nowMs: receivedMs,
    measure,
    insets,
    chrome,
  });
  const layers = useMemo(() => buildLayers(frame, colors), [frame, colors]);

  const onClick = (info: PickingInfo): void => {
    const file = pickedFile(info, frame);
    if (file !== null) {
      selectFile(file);
      return;
    }
    const hub = pickedHub(info, frame);
    if (hub !== null) moveTo(cameraOnHub(scene, hub, screen, insets));
  };

  return (
    <div className="map" ref={containerRef}>
      <div className="map-canvas">
        <DeckGL
          views={VIEW}
          viewState={viewState}
          onViewStateChange={({ viewState: next }) => {
            follow(next);
          }}
          layers={layers}
          onHover={(info) => {
            hoverFile(pickedFile(info, frame));
          }}
          onClick={onClick}
          getCursor={cursorFor}
        />
      </div>
      <MapStatus scene={scene} selected={selected} minStrength={minStrength} />
      <div className="map-controls" ref={controlsRef}>
        <Controls zoom={camera.zoom} onAction={act} />
        <Legend scene={scene} nowMs={receivedMs} />
      </div>
    </div>
  );
}
