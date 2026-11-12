// Measures the map on a generated repository of 100,000 files: the first load (from navigation to
// two drawn frames) and the frame rate over a scripted pan and zoom, sampled with
// requestAnimationFrame in the page. Prints both with the platform; checks no budget yet.
// Chromium runs on the machine's GPU (ANGLE on Direct3D 11 on Windows), since its default
// headless software renderer measures the CPU, not the map.
import { arch, cpus, platform, release } from 'node:os';
import { chromium, expect, test, type Page } from '@playwright/test';
import { largeFixture } from './fixtures';
import { MAP_URL, serveFixture } from './serve';

const FILE_COUNT = 100_000;
const GPU_ARGS = ['--use-angle=d3d11', '--enable-gpu', '--ignore-gpu-blocklist'];

/** Starts recording frame times in the page. */
async function startSampling(page: Page): Promise<void> {
  await page.evaluate(() => {
    const times: number[] = [];
    Reflect.set(window, '__frameTimes', times);
    const tick = (time: number): void => {
      times.push(time);
      if (Reflect.get(window, '__frameTimes') === times) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

/** Stops recording and returns the frame times. */
async function stopSampling(page: Page): Promise<number[]> {
  return page.evaluate(() => {
    const times: unknown = Reflect.get(window, '__frameTimes');
    Reflect.set(window, '__frameTimes', null);
    return Array.isArray(times)
      ? times.filter((time): time is number => typeof time === 'number')
      : [];
  });
}

/** Drags across the map and wheels in and out, over about four seconds. */
async function panAndZoom(page: Page): Promise<void> {
  await page.mouse.move(720, 480);
  await page.mouse.down();
  for (let step = 0; step < 60; step += 1) {
    await page.mouse.move(720 - step * 6, 480 - step * 3);
  }
  await page.mouse.up();
  for (let step = 0; step < 30; step += 1) {
    await page.mouse.wheel(0, step < 15 ? -120 : 120);
  }
  await page.mouse.down();
  for (let step = 0; step < 60; step += 1) {
    await page.mouse.move(360 + step * 6, 300 + step * 3);
  }
  await page.mouse.up();
}

function summarise(times: readonly number[]): string {
  const gaps = times.slice(1).map((time, i) => time - (times[i] ?? time));
  const seconds = ((times.at(-1) ?? 0) - (times[0] ?? 0)) / 1000;
  const sorted = [...gaps].sort((a, b) => a - b);
  const p95Gap = sorted[Math.floor(sorted.length * 0.95)] ?? 0;
  const mean = gaps.length / seconds;
  return `${gaps.length} frames in ${seconds.toFixed(2)} s: mean ${mean.toFixed(1)} fps, 5th percentile ${(1000 / p95Gap).toFixed(1)} fps (95th percentile frame ${p95Gap.toFixed(1)} ms), longest frame ${(sorted.at(-1) ?? 0).toFixed(1)} ms`;
}

/** Writes one line of the measurement to the test's output. */
function report(line: string): void {
  process.stdout.write(`${line}
`);
}

/** The GPU the page draws with, as WebGL names it. */
async function rendererOf(page: Page): Promise<string> {
  return page.evaluate(() => {
    const gl = document.createElement('canvas').getContext('webgl2');
    const info = gl?.getExtension('WEBGL_debug_renderer_info');
    return gl === null || info === undefined || info === null
      ? 'unknown'
      : String(gl.getParameter(info.UNMASKED_RENDERER_WEBGL));
  });
}

/** Flies to a file deep in the tree, where single files and their names are drawn. */
async function flyIntoFiles(page: Page): Promise<void> {
  await page.keyboard.press('/');
  await page.keyboard.type('pkg3/mod4');
  await expect(
    page.getByRole('list', { name: 'Search results' }).getByRole('button').first(),
  ).toBeVisible();
  await page.keyboard.press('Enter');
  await page.keyboard.press('Escape');
  await page.mouse.move(720, 480);
  for (let step = 0; step < 4; step += 1) await page.mouse.wheel(0, 120);
}

async function measure(label: string, args: string[]): Promise<void> {
  const browser = await chromium.launch({ args });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await serveFixture(page, largeFixture(FILE_COUNT));
  await page.goto(MAP_URL);
  await expect(page.getByTestId('summary-indexed')).toHaveText(/^100,000 files in/);
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  const firstLoadMs = await page.evaluate(() => performance.now());
  report(`[${label}] renderer: ${await rendererOf(page)}; browser ${browser.version()}`);
  report(`[${label}] first load: ${firstLoadMs.toFixed(0)} ms from navigation to two drawn frames`);
  await startSampling(page);
  await panAndZoom(page);
  report(`[${label}] pan and zoom over the whole map: ${summarise(await stopSampling(page))}`);
  await flyIntoFiles(page);
  await startSampling(page);
  await panAndZoom(page);
  report(`[${label}] pan and zoom among single files: ${summarise(await stopSampling(page))}`);
  expect(errors).toEqual([]);
  await browser.close();
}

test('100,000 files: first load and frame rate over a pan and zoom', async () => {
  test.setTimeout(300_000);
  const cpu = cpus()[0]?.model ?? 'unknown CPU';
  report(
    `platform: ${platform()} ${release()} ${arch()}, ${cpu} x${cpus().length}, viewport 1440x900`,
  );
  await measure('GPU', GPU_ARGS);
  await measure('software (headless default)', []);
});
