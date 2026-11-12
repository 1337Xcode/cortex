// Shared steps of the browser checks: open the map on a fixture and wait until it has drawn,
// collect console errors, and read pixels back from a screenshot (decoded by the page itself, so
// no image library is needed).
import { expect, type Page } from '@playwright/test';
import { parseGraph, type GraphSnapshot } from '../src/api/graph';
import type { Fixture } from './fixtures';
import { MAP_URL, serveFixture } from './serve';

/** What a page logged and asked that a check may want to look at. */
export interface PageRecord {
  /** Console errors and uncaught exceptions. */
  readonly errors: string[];
  /** Requests to any origin but the map's. */
  readonly foreign: string[];
}

/** Opens the map on `fixture` in `scheme` and waits for two drawn frames. */
export async function openMap(
  page: Page,
  fixture: Fixture,
  scheme: 'dark' | 'light' = 'dark',
): Promise<PageRecord> {
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  const foreign = await serveFixture(page, fixture);
  await page.emulateMedia({ colorScheme: scheme, reducedMotion: 'reduce' });
  await page.goto(MAP_URL);
  await expect(page.getByTestId('map-status')).toBeAttached();
  await nextFrames(page);
  return { errors, foreign };
}

/** Waits for two animation frames, so what React set is drawn. */
export async function nextFrames(page: Page): Promise<void> {
  await page.evaluate(
    () =>
      new Promise<void>((resolve) => {
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            resolve();
          });
        });
      }),
  );
}

/** The fixture's graph as the page's own guard reads it. */
export function snapshotOf(fixture: Fixture): GraphSnapshot {
  const result = parseGraph(fixture.graph);
  if (!result.ok) throw new Error(`the fixture breaks the contract: ${result.error.message}`);
  return result.value;
}

/** A rectangle of the page in CSS pixels. */
export interface Region {
  readonly x: number;
  readonly y: number;
  readonly width: number;
  readonly height: number;
}

/**
 * Counts the pixels of a screenshot region whose largest channel difference from `background`
 * (`#rrggbb`) is above `threshold`, and the box around them. With `color`, counts only pixels
 * within `threshold` of that colour instead.
 */
export async function inkIn(
  page: Page,
  region: Region,
  options: { readonly background: string; readonly threshold: number; readonly color?: string },
): Promise<{ count: number; box: Region | null }> {
  const png = (await page.screenshot({ clip: region })).toString('base64');
  return page.evaluate(
    async ({ png, region, options }) => {
      // Decoded from bytes, not fetched from a data: URL, which the page's policy refuses.
      const bytes = Uint8Array.from(atob(png), (char) => char.charCodeAt(0));
      const bitmap = await createImageBitmap(new Blob([bytes], { type: 'image/png' }));
      const canvas = new OffscreenCanvas(bitmap.width, bitmap.height);
      const context = canvas.getContext('2d');
      if (context === null) throw new Error('no 2D context');
      context.drawImage(bitmap, 0, 0);
      const { data } = context.getImageData(0, 0, bitmap.width, bitmap.height);
      const channels = (hex: string): number[] =>
        [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16));
      const reference = channels(options.color ?? options.background);
      let count = 0;
      let [minX, minY, maxX, maxY] = [Infinity, Infinity, -Infinity, -Infinity];
      for (let i = 0; i < data.length; i += 4) {
        const diff = Math.max(...reference.map((value, c) => Math.abs((data[i + c] ?? 0) - value)));
        const isInk =
          options.color === undefined ? diff > options.threshold : diff <= options.threshold;
        if (!isInk) continue;
        count += 1;
        const x = (i / 4) % bitmap.width;
        const y = Math.floor(i / 4 / bitmap.width);
        [minX, minY, maxX, maxY] = [
          Math.min(minX, x),
          Math.min(minY, y),
          Math.max(maxX, x),
          Math.max(maxY, y),
        ];
      }
      const box =
        count === 0
          ? null
          : {
              x: region.x + minX,
              y: region.y + minY,
              width: maxX - minX + 1,
              height: maxY - minY + 1,
            };
      return { count, box };
    },
    { png, region, options },
  );
}

/** Presses Tab until the focused element matches `selector`, at most 40 times. */
export async function tabTo(page: Page, selector: string): Promise<void> {
  for (let i = 0; i < 40; i += 1) {
    await page.keyboard.press('Tab');
    const isThere = await page.evaluate(
      (wanted) => document.activeElement?.matches(wanted) === true,
      selector,
    );
    if (isThere) return;
  }
  throw new Error(`Tab never reached ${selector}`);
}

/** Searches with the keyboard and picks the first result with Enter. */
export async function searchAndPick(page: Page, text: string): Promise<void> {
  await page.keyboard.press('/');
  await expect(page.getByRole('searchbox', { name: 'Search files and symbols' })).toBeFocused();
  await page.keyboard.type(text);
  await expect(
    page.getByRole('list', { name: 'Search results' }).getByRole('button').first(),
  ).toBeVisible();
  await page.keyboard.press('Enter');
}

/** The bottom edge of the top bar in CSS pixels. */
export async function headerBottom(page: Page): Promise<number> {
  return page
    .locator('header.topbar')
    .evaluate((element) => Math.ceil(element.getBoundingClientRect().bottom));
}
