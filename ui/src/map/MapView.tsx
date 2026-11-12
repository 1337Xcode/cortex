// The map: the deck.gl canvas, what pointing at and clicking its nodes does, and the controls, the
// view switch and the legend beside it. Hovering a file highlights it and its neighbours; every
// mark, badge, link and live marker under the pointer says what it is, with its number; clicking a
// file selects it; clicking a folder flies in until it opens. Live sessions are drawn over the map
// while the live view is open; a frame never moves the camera.
import type { PickingInfo } from '@deck.gl/core';
import DeckGL from '@deck.gl/react';
import { useMemo, useRef, useState, type ReactElement } from 'react';
import type { LiveSession } from '../api/live';
import { Controls } from '../controls/Controls';
import { Legend } from '../controls/Legend';
import { ViewSwitch } from '../controls/ViewSwitch';
import { useMapStore, type LiveStatus } from '../state/store';
import { paintView } from '../views/paint';
import { viewNamed } from '../views/registry';
import { fontMeasurer } from '../text/measure';
import { useIsNarrow, useThemeName } from '../theme/media';
import { CANVAS_SIZE, FONT, SPACE, THEMES } from '../theme/tokens';
import { cameraOnHub, type Insets } from './camera';
import { buildLayers, LAYER_ID, MAP_VIEW, type FrameBuffers } from './layers';
import { MapStatus } from './MapStatus';
import type { MeasureText } from './labelCandidates';
import type { ScreenBox } from './labels';
import { LivePaused } from './LivePaused';
import { explainPick } from './mapTooltip';
import type { Scene } from './scene';
import { useFrame } from './useFrame';
import { useMapCamera } from './useMapCamera';
import { useElementBox, useScreenSize } from './useScreenSize';

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

/** Label widths in the canvas labels' family, rounded up so a label's box always holds it. */
function labelMeasurer(): MeasureText {
  const measure = fontMeasurer();
  return (text, sizePx, weight) => Math.ceil(measure(text, `${weight} ${sizePx}px ${FONT.sans}`));
}

/** What the pointer is on: where, and the sentence that explains it. */
interface Pointed {
  readonly x: number;
  readonly y: number;
  readonly text: string;
}

/**
 * The explanation beside the pointer, kept off the pointer itself and on the side of it with the
 * more room, so it never runs off the screen.
 */
function PointerTooltip(props: {
  pointed: Pointed | null;
  screen: { width: number; height: number };
}): ReactElement | null {
  const { pointed, screen } = props;
  if (pointed === null) return null;
  const isLeft = pointed.x > screen.width / 2;
  const isAbove = pointed.y > screen.height / 2;
  return (
    <div
      className="map-tooltip"
      role="tooltip"
      data-left={isLeft}
      data-above={isAbove}
      style={{
        left: isLeft ? pointed.x - SPACE.md : pointed.x + SPACE.md,
        top: isAbove ? pointed.y - SPACE.md : pointed.y + SPACE.md,
      }}
    >
      {pointed.text}
    </div>
  );
}

/** No session: what is drawn while the live view connects or is paused. */
const NO_SESSIONS: readonly LiveSession[] = [];

/** The sessions to draw: the last frame's while the live view is open, none otherwise. */
function drawnSessions(live: LiveStatus): readonly LiveSession[] {
  return live.status === 'open' ? live.state.sessions : NO_SESSIONS;
}

/**
 * The chrome's insets, and on a narrow screen the controls' rows too: there they stack at the
 * bottom of the canvas, and the camera keeps what it frames clear of them.
 */
function useCanvasInsets(insets: Insets, controls: ScreenBox): Insets {
  const isNarrow = useIsNarrow();
  return useMemo(() => {
    if (!isNarrow || controls.height === 0) return insets;
    return { ...insets, bottom: Math.max(insets.bottom, controls.height + SPACE.sm) };
  }, [isNarrow, insets, controls.height]);
}

/** The map of one snapshot. Mount it again (a new `key`) for a new snapshot. */
export function MapView({
  scene,
  receivedMs,
  insets: chromeInsets,
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
  const viewName = useMapStore((state) => state.view);
  const live = useMapStore((state) => state.live);
  const sessions = drawnSessions(live);
  const view = useMemo(
    () => paintView(viewNamed(viewName), scene, receivedMs),
    [viewName, scene, receivedMs],
  );
  const hoverFile = useMapStore((state) => state.hoverFile);
  const selectFile = useMapStore((state) => state.selectFile);
  const containerRef = useRef<HTMLDivElement>(null);
  const screen = useScreenSize(containerRef);
  const controlsRef = useRef<HTMLDivElement>(null);
  const controlsBox = useElementBox(controlsRef);
  const insets = useCanvasInsets(chromeInsets, controlsBox);
  const chrome = useMemo(() => [controlsBox], [controlsBox]);
  const measure = useMemo(() => labelMeasurer(), []);
  const [pointed, setPointed] = useState<Pointed | null>(null);
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
    view,
    sessions,
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
          views={MAP_VIEW}
          viewState={viewState}
          onViewStateChange={({ viewState: next }) => {
            follow(next);
          }}
          layers={layers}
          onHover={(info) => {
            hoverFile(pickedFile(info, frame));
            const text = explainPick(info, frame, scene, receivedMs);
            setPointed(text === null ? null : { x: info.x, y: info.y, text });
          }}
          pickingRadius={CANVAS_SIZE.pickRadiusPx}
          onClick={onClick}
          getCursor={cursorFor}
        />
      </div>
      <PointerTooltip pointed={pointed} screen={screen} />
      <MapStatus scene={scene} selected={selected} minStrength={minStrength} />
      <div className="map-controls" ref={controlsRef}>
        <Controls zoom={camera.zoom} onAction={act} />
        <ViewSwitch />
        <Legend scene={scene} nowMs={receivedMs} view={view} sessions={frame.live.sessions} />
        {live.status === 'paused' ? <LivePaused /> : null}
      </div>
    </div>
  );
}
