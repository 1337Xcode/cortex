// Assembles one frame's buffers with each piece rebuilt only when its own inputs change: the level
// of detail only when a hub crosses its threshold, the files and links only when the level, the
// focus, the threshold, the view or the theme change, the live markers when a live frame arrives
// or the zoom changes their size, and the grid and labels as the camera moves. A live frame
// rebuilds only the markers and the labels; the layout and the camera never depend on it.
import { useMemo } from 'react';
import type { LiveSession } from '../api/live';
import { sessionName } from '../text/liveWords';
import type { SessionNames } from '../text/sessionNames';
import type { ViewPaint } from '../views/paint';
import { agentFamily } from './agents';
import { areBadgesShown, encodeBadges } from './badges';
import { levelOfDetail, type LevelOfDetail } from '../layout/lod';
import { RING_SPACING } from '../layout/constants';
import { CANVAS_SIZE, HUB_LINK_DASH_PX, LINK_DENSITY, type ThemeColors } from '../theme/tokens';
import type { Camera, Insets } from './camera';
import { encodeFiles } from './fileEncoding';
import { encodeFileMarks } from './fileMarks';
import type { FrameStyle } from './frameStyle';
import { gridDots } from './grid';
import { neighbourhoodOf } from './highlight';
import {
  encodeCollapsedHubs,
  encodeFileDiscs,
  encodeHubLinks,
  encodeHubSquares,
  encodeSpokes,
  focusHubs,
} from './hubEncoding';
import {
  chooseLabels,
  type CanvasLabel,
  type LabelInputs,
  type MarkerLabel,
  type MeasureText,
} from './labelCandidates';
import type { ScreenBox, ScreenSize } from './labels';
import type { FrameBuffers } from './layers';
import { encodeLinks, type LineBuffers } from './linkEncoding';
import { encodeLive, type LiveBuffers } from './liveEncoding';
import { boxKey, nearBox, overviewLinks, prominentLinks, type WorldBox } from './linkProminence';
import type { Scene } from './scene';
import { computeVisibility, type Visibility } from './visibility';

/** What one frame is drawn from. */
export interface FrameInputs {
  readonly scene: Scene;
  readonly camera: Camera;
  readonly screen: ScreenSize;
  readonly colors: ThemeColors;
  readonly selected: number | null;
  readonly hovered: number | null;
  readonly minStrength: number;
  readonly nowMs: number;
  readonly measure: MeasureText;
  /** The canvas edges the page's chrome covers; labels stay out of them. */
  readonly insets: Insets;
  /** Boxes of the page's controls over the canvas; labels stay off them. */
  readonly chrome: readonly ScreenBox[];
  /** The view the files are coloured by. */
  readonly view: ViewPaint;
  /** The live sessions to mark; empty while none runs or the live view is paused. */
  readonly sessions: readonly LiveSession[];
  /** The names the page gives its sessions, which the markers' labels use. */
  readonly sessionNames: SessionNames;
}

/** The part of the screen the chrome leaves clear. */
function clearArea(screen: ScreenSize, insets: Insets): ScreenBox {
  return {
    left: insets.left,
    top: insets.top,
    width: Math.max(screen.width - insets.left - insets.right, 0),
    height: Math.max(screen.height - insets.top - insets.bottom, 0),
  };
}

/** An exact text key of a level of detail's hub states, so equal levels share one memo entry. */
function levelKey(lod: LevelOfDetail): string {
  const chunks: string[] = [];
  // In chunks: spreading a very large array into one call overflows the stack.
  for (let at = 0; at < lod.state.length; at += 8192) {
    chunks.push(String.fromCharCode(...lod.state.subarray(at, at + 8192)));
  }
  return chunks.join('');
}

/** A text key of a label list: equal keys draw the same labels the same way. */
function labelsKey(labels: readonly CanvasLabel[]): string {
  return labels
    .map(
      (label) => `${label.key}:${label.isDimmed ? 1 : 0}:${label.side}:${label.offset.join(',')}`,
    )
    .join('|');
}

/** The level of detail at `zoom`, the same object while no hub changes state. */
function useLevel(scene: Scene, zoom: number): LevelOfDetail {
  const next = levelOfDetail(scene.layout, zoom);
  // Reused while no hub changes state, so everything that depends on it is reused too.
  const signature = levelKey(next);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the signature stands for the level
  return useMemo(() => next, [scene, signature]);
}

/** The labels to draw, the same array while the same labels are chosen. */
function useLabels(inputs: LabelInputs): readonly CanvasLabel[] {
  const { scene, visibility, camera, screen, area, chrome, neighbourhood, markedHubs, measure } =
    inputs;
  const { markers } = inputs;
  const chosen = useMemo(
    () =>
      chooseLabels({
        scene,
        visibility,
        camera,
        screen,
        area,
        chrome,
        neighbourhood,
        markedHubs,
        measure,
        markers,
      }),
    [scene, visibility, camera, screen, area, chrome, neighbourhood, markedHubs, measure, markers],
  );
  // Reused while the same labels are chosen, so the text layers are not rebuilt as the camera moves.
  const signature = labelsKey(chosen);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the signature stands for the list
  return useMemo(() => chosen, [signature]);
}

/**
 * The learned links, re-encoded when the drawn set, the focus or the box near the screen changes
 * (the box moves in quarter-screen steps, so panning re-encodes a few times, not every frame).
 * On the overview (files not drawn one by one, no file pointed at or selected) only the strongest
 * few are drawn.
 */
function useProminentLinks(
  scene: Scene,
  visibility: Visibility,
  style: FrameStyle,
  box: WorldBox,
): LineBuffers {
  const key = boxKey(box);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the key stands for the box
  const stableBox = useMemo(() => box, [key]);
  const { minStrength } = style;
  const isOverview = !visibility.areFilesShown && style.neighbourhood === null;
  const prominent = useMemo(() => {
    if (!isOverview) {
      return prominentLinks(scene, visibility, {
        minStrength,
        box: stableBox,
        limits: LINK_DENSITY,
      });
    }
    const count = LINK_DENSITY.overviewStrongest;
    return overviewLinks(scene, visibility, { minStrength, box: stableBox, count });
  }, [scene, visibility, minStrength, stableBox, isOverview]);
  return useMemo(
    () => encodeLinks(scene, visibility, style, prominent),
    [scene, visibility, style, prominent],
  );
}

/** The live sessions' buffers and their markers' labels, at `scale` pixels per world unit. */
function useLive(
  scene: Scene,
  visibility: Visibility,
  sessions: readonly LiveSession[],
  view: { readonly colors: ThemeColors; readonly scale: number },
  names: SessionNames,
): { readonly live: LiveBuffers; readonly markers: readonly MarkerLabel[] } {
  const { colors, scale } = view;
  const live = useMemo(
    () => encodeLive(scene, visibility, sessions, { colors, scale }),
    [scene, visibility, sessions, colors, scale],
  );
  const markers = useMemo(
    () =>
      live.spots.map((spot): MarkerLabel => {
        const session = live.sessions[spot.session];
        return {
          key: `s${session?.id ?? ''}`,
          text: session === undefined ? '' : sessionName(session, names),
          family: agentFamily(session?.agent ?? ''),
          file: spot.file,
          worldX: spot.worldX,
          worldY: spot.worldY,
          radiusPx: spot.radiusPx,
        };
      }),
    [live, names],
  );
  return { live, markers };
}

/** The frame's buffers for the deck.gl layers. */
export function useFrame(inputs: FrameInputs): FrameBuffers {
  const { scene, camera, screen, colors, selected, hovered, minStrength, nowMs } = inputs;
  const { view } = inputs;
  const lod = useLevel(scene, camera.zoom);
  const areFilesShown = 2 ** camera.zoom * RING_SPACING >= CANVAS_SIZE.fileMinRingSpacingPx;
  const visibility = useMemo(
    () => computeVisibility(scene.layout, lod, areFilesShown),
    [scene, lod, areFilesShown],
  );
  const focus = hovered ?? selected;
  const neighbourhood = useMemo(
    () => neighbourhoodOf(scene, focus, minStrength),
    [scene, focus, minStrength],
  );
  const style = useMemo<FrameStyle>(
    () => ({ colors, nowMs, neighbourhood, selected, minStrength, view }),
    [colors, nowMs, neighbourhood, selected, minStrength, view],
  );
  const scale = 2 ** camera.zoom;
  const isBadged = areBadgesShown(visibility, scale);
  const marked = useMemo(() => focusHubs(scene, visibility, style), [scene, visibility, style]);
  const geometry = useMemo(
    () => ({
      hubLinks: encodeHubLinks(scene, visibility, style, marked, HUB_LINK_DASH_PX),
      spokes: encodeSpokes(scene, visibility, style),
      collapsed: encodeCollapsedHubs(scene, visibility, style, marked),
      fileDiscs: encodeFileDiscs(scene, visibility, style, marked),
      files: encodeFiles(scene, visibility, style),
      fileMarks: encodeFileMarks(scene, visibility, style),
      badges: encodeBadges(scene, visibility, style, isBadged),
      hubs: encodeHubSquares(scene, visibility, style, marked),
    }),
    [scene, visibility, style, marked, isBadged],
  );
  const { live, markers } = useLive(
    scene,
    visibility,
    inputs.sessions,
    { colors, scale },
    inputs.sessionNames,
  );
  const links = useProminentLinks(scene, visibility, style, nearBox(camera, screen));
  const grid = useMemo(() => gridDots(camera, screen), [camera, screen]);
  const area = useMemo(() => clearArea(screen, inputs.insets), [screen, inputs.insets]);
  const { measure, chrome } = inputs;
  const labels = useLabels({
    scene,
    visibility,
    camera,
    screen,
    area,
    chrome,
    neighbourhood,
    markedHubs: marked,
    measure,
    markers,
  });
  return useMemo(
    () => ({ ...geometry, links, live, grid, labels }),
    [geometry, links, live, grid, labels],
  );
}
