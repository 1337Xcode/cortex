// Every colour token, converted by the map's own OKLCH code, against the browser's own colour
// engine: Chromium paints the token's CSS value into a canvas, and the pixel it paints must match
// the channels the canvas layers are given, to within one step of 255. So the page's CSS and the
// deck.gl canvas show the same colour for every token, in both themes.
import { expect, test } from '@playwright/test';
import { cssOf, rgbaOf } from '../src/theme/oklch';
import { colorRoles, THEMES } from '../src/theme/tokens';

for (const [theme, colors] of Object.entries(THEMES)) {
  test(`${theme} theme: the browser paints each token as the canvas draws it`, async ({ page }) => {
    const roles = colorRoles(colors);
    const painted = await page.evaluate(
      (values) => {
        const canvas = document.createElement('canvas');
        canvas.width = 1;
        canvas.height = 1;
        const context = canvas.getContext('2d', { colorSpace: 'srgb' });
        if (context === null) throw new Error('no 2D context');
        return values.map((value) => {
          context.clearRect(0, 0, 1, 1);
          context.fillStyle = value;
          context.fillRect(0, 0, 1, 1);
          return Array.from(context.getImageData(0, 0, 1, 1).data.slice(0, 3));
        });
      },
      roles.map(([, color]) => cssOf(color)),
    );
    for (const [index, [name, color]] of roles.entries()) {
      const ours = rgbaOf(color).slice(0, 3);
      const theirs = painted[index] ?? [];
      for (const channel of [0, 1, 2]) {
        const difference = Math.abs((ours[channel] ?? 0) - (theirs[channel] ?? -9));
        expect(difference, `${theme} ${name} channel ${channel}`).toBeLessThanOrEqual(1);
      }
    }
  });
}
