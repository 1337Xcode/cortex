// The faint dot grid behind the graph: one dot every `gridPitchPx` pixels, anchored to the world so
// it pans with the canvas. Only the dots on screen are generated, so its cost depends on the
// screen size and never on the repository.
import { CANVAS_SIZE } from '../theme/tokens';
import type { Camera } from './camera';
import type { ScreenSize } from './labels';

/** The dots on screen as world positions, x then y per dot. */
export function gridDots(camera: Camera, screen: ScreenSize): Float32Array {
  const scale = 2 ** camera.zoom;
  const pitch = CANVAS_SIZE.gridPitchPx / scale;
  const halfWidth = screen.width / 2 / scale;
  const halfHeight = screen.height / 2 / scale;
  const firstX = Math.floor((camera.x - halfWidth) / pitch);
  const firstY = Math.floor((camera.y - halfHeight) / pitch);
  const columns = Math.ceil(screen.width / CANVAS_SIZE.gridPitchPx) + 2;
  const rows = Math.ceil(screen.height / CANVAS_SIZE.gridPitchPx) + 2;
  const dots = new Float32Array(columns * rows * 2);
  let at = 0;
  for (let row = 0; row < rows; row += 1) {
    for (let column = 0; column < columns; column += 1) {
      dots[at] = (firstX + column) * pitch;
      dots[at + 1] = (firstY + row) * pitch;
      at += 2;
    }
  }
  return dots;
}
