// The camera: where the view looks and how far in it is, and the moves the controls make (fit the
// graph, centre, zoom in steps of 1.3, pan, fly to a file or a hub). Pure functions of the view,
// the screen size and the scene, so every move can be tested without a browser.
import { COLLAPSE_BELOW_RADIUS_PX } from '../layout/constants';
import { CAMERA } from '../theme/tokens';
import type { Scene } from './scene';
import type { ScreenSize } from './labels';

/** The view: the world point at the centre of the screen and the zoom (2^zoom pixels per unit). */
export interface Camera {
  readonly x: number;
  readonly y: number;
  readonly zoom: number;
}

/** The zoom change of one − or + step: a factor of 1.3 in scale. */
export const ZOOM_STEP = Math.log2(CAMERA.zoomStepFactor);

/** Keeps a zoom inside the camera's limits. */
export function clampZoom(zoom: number): number {
  return Math.min(CAMERA.maxZoom, Math.max(CAMERA.minZoom, zoom));
}

/** The zoom shown to the person, as a whole percentage (100% is one pixel per world unit). */
export function zoomPercent(zoom: number): number {
  return Math.round(2 ** zoom * 100);
}

/**
 * The zoom as the controls show it: whole percent, with one decimal below 10%, and "<0.1%" for a
 * zoom that one decimal would round to 0, which the scale never is.
 */
export function formatZoom(zoom: number): string {
  const percent = 2 ** zoom * 100;
  if (percent < 0.05) return '<0.1%';
  return percent < 10 ? `${percent.toFixed(1)}%` : `${Math.round(percent)}%`;
}

/** Projects a world point to screen pixels (y grows downwards, as deck.gl's orthographic view draws it). */
export function toScreen(
  camera: Camera,
  screen: ScreenSize,
  x: number,
  y: number,
): [number, number] {
  const scale = 2 ** camera.zoom;
  return [(x - camera.x) * scale + screen.width / 2, (y - camera.y) * scale + screen.height / 2];
}

/** Screen edges covered by the page's chrome, in pixels: the camera centres on what stays visible. */
export interface Insets {
  readonly top: number;
  readonly right: number;
  readonly bottom: number;
  readonly left: number;
}

/** No chrome over the canvas. */
export const NO_INSETS: Insets = { top: 0, right: 0, bottom: 0, left: 0 };

/** The visible area's width and height in pixels, never below 1. */
function visibleSize(screen: ScreenSize, insets: Insets): [number, number] {
  return [
    Math.max(screen.width - insets.left - insets.right, 1),
    Math.max(screen.height - insets.top - insets.bottom, 1),
  ];
}

/** The camera that puts world point (x, y) at the centre of the visible area, at `zoom`. */
export function lookAt(x: number, y: number, zoom: number, insets: Insets): Camera {
  const clamped = clampZoom(zoom);
  const scale = 2 ** clamped;
  // The visible centre sits this far from the canvas centre, in pixels.
  const shiftX = (insets.left - insets.right) / 2;
  const shiftY = (insets.top - insets.bottom) / 2;
  return { x: x - shiftX / scale, y: y - shiftY / scale, zoom: clamped };
}

/**
 * The camera that shows the whole graph in the visible area with `CAMERA.fitPaddingPx` to spare,
 * zoomed in no further than `CAMERA.maxFitZoom` so a small repository is not blown up.
 */
export function fitCamera(scene: Scene, screen: ScreenSize, insets: Insets): Camera {
  const { minX, minY, maxX, maxY } = scene.bounds;
  const [width, height] = visibleSize(screen, insets);
  const pad = CAMERA.fitPaddingPx * 2;
  const scale = Math.min(
    Math.max(width - pad, 1) / Math.max(maxX - minX, 1),
    Math.max(height - pad, 1) / Math.max(maxY - minY, 1),
  );
  const zoom = Math.min(Math.log2(scale), CAMERA.maxFitZoom);
  return lookAt((minX + maxX) / 2, (minY + maxY) / 2, zoom, insets);
}

/** The camera zoomed one step in (`direction` 1) or out (-1), keeping its centre. */
export function stepZoom(camera: Camera, direction: 1 | -1): Camera {
  return { ...camera, zoom: clampZoom(camera.zoom + direction * ZOOM_STEP) };
}

/** The camera panned by a distance in screen pixels. */
export function panBy(camera: Camera, dxPx: number, dyPx: number): Camera {
  const scale = 2 ** camera.zoom;
  return { ...camera, x: camera.x + dxPx / scale, y: camera.y + dyPx / scale };
}

/** The least zoom at which `hub` draws open (its covering circle at least the collapse size). */
function openZoomOf(scene: Scene, hub: number): number {
  const radius = Math.max(scene.layout.hubRadius[hub] ?? 1, 1e-6);
  return Math.log2(COLLAPSE_BELOW_RADIUS_PX / radius);
}

/** The camera on file `id`, zoomed in far enough that its hub is open and its name drawn. */
export function cameraOnFile(scene: Scene, camera: Camera, id: number, insets: Insets): Camera {
  const hub = scene.layout.fileHub[id] ?? 0;
  // A small margin above the opening zoom, so rounding never leaves the hub just collapsed.
  const zoom = Math.max(camera.zoom, CAMERA.focusZoom, openZoomOf(scene, hub) + 0.25);
  return lookAt(scene.layout.fileX[id] ?? 0, scene.layout.fileY[id] ?? 0, zoom, insets);
}

/** The camera that fills the visible area with `hub`'s whole sub-tree. */
export function cameraOnHub(scene: Scene, hub: number, screen: ScreenSize, insets: Insets): Camera {
  const radius = Math.max(scene.layout.hubRadius[hub] ?? 1, 1e-6);
  const [width, height] = visibleSize(screen, insets);
  const room = Math.max(Math.min(width, height) / 2 - CAMERA.fitPaddingPx, 1);
  const zoom = Math.max(Math.log2(room / radius), openZoomOf(scene, hub) + 0.25);
  return lookAt(scene.layout.hubX[hub] ?? 0, scene.layout.hubY[hub] ?? 0, zoom, insets);
}
