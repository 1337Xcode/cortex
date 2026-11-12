// Assembles one frame's buffers with each piece rebuilt only when its own inputs change: the level
// of detail only when a hub crosses its threshold, the files and links only when the level, the
// focus, the threshold or the theme change, and the grid and labels as the camera moves.
import { useMemo } from 'react';
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
  type MeasureText,
} from './labelCandidates';
import type { ScreenBox, ScreenSize } from './labels';
import type { FrameBuffers } from './layers';
import { encodeLinks, type LineBuffers } from './linkEncoding';
import { boxKey, nearBox, prominentLinks, type WorldBox } from './linkProminence';
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
      }),
    [scene, visibility, camera, screen, area, chrome, neighbourhood, markedHubs, measure],
  );
  // Reused while the same labels are chosen, so the text layers are not rebuilt as the camera moves.
  const signature = labelsKey(chosen);
  // eslint-disable-next-line react-hooks/exhaustive-deps -- the signature stands for the list
  return useMemo(() => chosen, [signature]);
}

/**
 * The learned links, re-encoded when the drawn set, the focus or the box near the screen changes
 * (the box moves in quarter-screen steps, so panning re-encodes a few times, not every frame).
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
  const prominent = useMemo(
    () => prominentLinks(scene, visibility, { minStrength, box: stableBox, limits: LINK_DENSITY }),
    [scene, visibility, minStrength, stableBox],
  );
  return useMemo(
    () => encodeLinks(scene, visibility, style, prominent),
    [scene, visibility, style, prominent],
  );
}

/** The frame's buffers for the deck.gl layers. */
export function useFrame(inputs: FrameInputs): FrameBuffers {
  const { scene, camera, screen, colors, selected, hovered, minStrength, nowMs } = inputs;
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
    () => ({ colors, nowMs, neighbourhood, selected, minStrength }),
    [colors, nowMs, neighbourhood, selected, minStrength],
  );
  const marked = useMemo(() => focusHubs(scene, visibility, style), [scene, visibility, style]);
  const geometry = useMemo(
    () => ({
      hubLinks: encodeHubLinks(scene, visibility, style, marked, HUB_LINK_DASH_PX),
      spokes: encodeSpokes(scene, visibility, style),
      collapsed: encodeCollapsedHubs(scene, visibility, style, marked),
      fileDiscs: encodeFileDiscs(scene, visibility, style, marked),
      files: encodeFiles(scene, visibility, style),
      fileMarks: encodeFileMarks(scene, visibility, style),
      hubs: encodeHubSquares(scene, visibility, style, marked),
    }),
    [scene, visibility, style, marked],
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
  });
  return useMemo(() => ({ ...geometry, links, grid, labels }), [geometry, links, grid, labels]);
}
