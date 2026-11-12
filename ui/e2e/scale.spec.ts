// Measures the map on a generated repository of 100,000 files: the first load (from navigation to
// two drawn frames) in five fresh browsers, each with nothing cached, and the frame rate over a
// scripted pan and zoom, sampled with requestAnimationFrame in the page. Prints them with the
// platform. When CORTEX_MAP_SCALE_FILE names a file, writes the GPU's numbers there as JSON for
// `cargo xtask verify --full`, which checks them against the map's budgets (2 s, 45 fps).
// Chromium runs on the machine's GPU through ANGLE (Direct3D 11 on Windows, Metal on macOS, its
// default elsewhere), since its default headless software renderer measures the CPU, not the map.
import { mkdirSync, writeFileSync } from 'node:fs';
import { arch, cpus, platform, release } from 'node:os';
import { dirname } from 'node:path';
import { chromium, expect, test, type Page } from '@playwright/test';
import { largeFixture, type Fixture } from './fixtures';
import { MAP_URL, serveFixture } from './serve';

const FILE_COUNT = 100_000;

/** Fresh browsers whose first load is measured; the budget holds for the slowest. */
const FRESH_LOADS = 5;

/** Chromium's flags for drawing with the GPU on this platform. */
function gpuArgs(): string[] {
  const angle: Partial<Record<NodeJS.Platform, string>> = { win32: 'd3d11', darwin: 'metal' };
  const backend = angle[platform()];
  return [
    ...(backend === undefined ? [] : [`--use-angle=${backend}`]),
    '--enable-gpu',
    '--ignore-gpu-blocklist',
  ];
}

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

/** The frame times' summary line and their 5th-percentile frame rate. */
function framesOf(times: readonly number[]): { line: string; p5Fps: number } {
  const gaps = times.slice(1).map((time, i) => time - (times[i] ?? time));
  const seconds = ((times.at(-1) ?? 0) - (times[0] ?? 0)) / 1000;
  const sorted = [...gaps].sort((a, b) => a - b);
  const p95Gap = sorted[Math.floor(sorted.length * 0.95)] ?? 0;
  const mean = gaps.length / seconds;
  const p5Fps = p95Gap > 0 ? 1000 / p95Gap : 0;
  const line = `${gaps.length} frames in ${seconds.toFixed(2)} s: mean ${mean.toFixed(1)} fps, 5th percentile ${p5Fps.toFixed(1)} fps (95th percentile frame ${p95Gap.toFixed(1)} ms), longest frame ${(sorted.at(-1) ?? 0).toFixed(1)} ms`;
  return { line, p5Fps };
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

/** What one browser measured. */
interface Measured {
  readonly renderer: string;
  readonly firstLoadMs: number;
  /** The 5th-percentile frame rate of each pan and zoom, when the browser panned at all. */
  readonly p5Fps: readonly number[];
}

/** Opens the map in a fresh browser; with `isPanned`, also pans and zooms over it and among files. */
async function measure(
  label: string,
  args: string[],
  fixture: Fixture,
  isPanned: boolean,
): Promise<Measured> {
  const browser = await chromium.launch({ args });
  const page = await browser.newPage({ viewport: { width: 1440, height: 900 } });
  const errors: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  await serveFixture(page, fixture);
  await page.goto(MAP_URL);
  await expect(page.getByTestId('summary-indexed')).toHaveText(/^100,000 files in/);
  await page.evaluate(
    () => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))),
  );
  const firstLoadMs = await page.evaluate(() => performance.now());
  const renderer = await rendererOf(page);
  report(`[${label}] renderer: ${renderer}; browser ${browser.version()}`);
  report(`[${label}] first load: ${firstLoadMs.toFixed(0)} ms from navigation to two drawn frames`);
  const p5Fps: number[] = [];
  if (isPanned) {
    for (const where of ['over the whole map', 'among single files']) {
      if (where === 'among single files') await flyIntoFiles(page);
      await startSampling(page);
      await panAndZoom(page);
      const frames = framesOf(await stopSampling(page));
      report(`[${label}] pan and zoom ${where}: ${frames.line}`);
      p5Fps.push(frames.p5Fps);
    }
  }
  expect(errors).toEqual([]);
  await browser.close();
  return { renderer, firstLoadMs, p5Fps };
}

/** Writes the GPU's numbers where `cargo xtask verify --full` reads them, when it asked. */
function writeRecord(platformLine: string, runs: readonly Measured[]): void {
  const path = process.env['CORTEX_MAP_SCALE_FILE'];
  if (path === undefined || path === '') return;
  mkdirSync(dirname(path), { recursive: true });
  const record = {
    platform: platformLine,
    renderer: runs[0]?.renderer ?? 'unknown',
    first_load_ms: runs.map((run) => Math.round(run.firstLoadMs)),
    pan_zoom_p5_fps: runs.flatMap((run) => run.p5Fps),
  };
  writeFileSync(path, JSON.stringify(record));
}

test('100,000 files: first load and frame rate over a pan and zoom', async () => {
  test.setTimeout(420_000);
  const cpu = cpus()[0]?.model ?? 'unknown CPU';
  const platformLine = `${platform()} ${release()} ${arch()}, ${cpu} x${cpus().length}`;
  report(`platform: ${platformLine}, viewport 1440x900`);
  const fixture = largeFixture(FILE_COUNT);
  const runs: Measured[] = [];
  for (let run = 1; run <= FRESH_LOADS; run += 1) {
    runs.push(await measure(`GPU ${run}`, gpuArgs(), fixture, run === FRESH_LOADS));
  }
  writeRecord(platformLine, runs);
  await measure('software (headless default)', [], fixture, true);
});
