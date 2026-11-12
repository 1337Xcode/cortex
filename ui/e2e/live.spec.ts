// The live view in a browser, its WebSocket answered by the stand-in in serve.ts: frames move the
// markers and never the camera; the overview's live section lists the sessions in rows of five,
// each flying to its newest file; a refused frame is dropped and the last state kept; a lost
// connection says the live view is paused, hides the markers, and comes back by itself. Markers
// are found by their agent's colour in the canvas's pixels.
import { expect, test, type Page } from '@playwright/test';
import { fitCamera, toScreen } from '../src/map/camera';
import { buildScene } from '../src/map/scene';
import { DARK_COLORS } from '../src/theme/tokens';
import { AGENTS, liveSession, NO_SESSIONS, smallFixture, type Json } from './fixtures';
import {
  chromeInsets,
  inkIn,
  nextFrames,
  openMap,
  snapshotOf,
  type PageRecord,
  type Region,
} from './helpers';
import { liveChannel } from './serve';

const VIEWPORT = { width: 1440, height: 900 };
const NOW_MS = Date.now();
const FIXTURE = smallFixture(NOW_MS);
const SCENE = buildScene(snapshotOf(FIXTURE));

let record: PageRecord = { errors: [], foreign: [] };

test.afterEach(() => {
  expect(record.errors, 'console errors').toEqual([]);
  expect(record.foreign, 'requests to other origins').toEqual([]);
});

/** A frame of one Claude Code session whose files, newest first, are `files`. */
function claudeOn(files: readonly number[]): Json {
  const pairs = files.map((file, index): [number, string] => [file, index === 0 ? 'edit' : 'read']);
  return { version: 1, sessions: [liveSession('3f2a9c1e', AGENTS.claude, pairs, NOW_MS)] };
}

/** The 50-pixel square around file `id` where the fitted map draws it. */
async function around(page: Page, id: number): Promise<Region> {
  const camera = fitCamera(SCENE, VIEWPORT, await chromeInsets(page, false));
  const [x, y] = toScreen(
    camera,
    VIEWPORT,
    SCENE.layout.fileX[id] ?? 0,
    SCENE.layout.fileY[id] ?? 0,
  );
  return { x: x - 25, y: y - 25, width: 50, height: 50 };
}

/** How many pixels of `region` are drawn in the Claude marker's rose. */
async function rose(page: Page, region: Region): Promise<number> {
  const colors = {
    background: DARK_COLORS.background,
    color: DARK_COLORS.agents.claude,
    threshold: 30,
  };
  return (await inkIn(page, region, colors)).count;
}

/** Where file `id`'s circle is drawn, as the box of its ink, and the zoom the map shows. */
async function cameraMark(page: Page, id: number): Promise<{ box: Region | null; zoom: string }> {
  const ink = await inkIn(page, await around(page, id), {
    background: DARK_COLORS.background,
    threshold: 60,
  });
  return { box: ink.box, zoom: (await page.locator('output.zoom-value').textContent()) ?? '' };
}

test('frames move the marker from file to file and never move the camera', async ({ page }) => {
  record = await openMap(page, { ...FIXTURE, live: claudeOn([4]) });
  const book = await around(page, 4);
  const payment = await around(page, 6);
  await expect.poll(() => rose(page, book)).toBeGreaterThan(30);
  expect(await rose(page, payment)).toBe(0);
  // src/lib.rs (3) has no marker in any frame; its circle must stay where it is.
  const before = await cameraMark(page, 3);
  const live = liveChannel(page);
  // Six frames back and forth, as fast as the test can send them, then the last one settles.
  for (const files of [[6], [4], [6], [4], [6], [6, 4]]) live.push(claudeOn(files));
  await expect.poll(() => rose(page, payment)).toBeGreaterThan(30);
  await nextFrames(page);
  expect(await cameraMark(page, 3)).toEqual(before);
  // The marker left book.rs; only its trail's faint dot stays there.
  expect(await rose(page, book)).toBeLessThan(30);
  await expect(page.getByTestId('map-status')).toHaveText('No file is highlighted.');
});

test('a marker explains its session under the pointer', async ({ page }) => {
  record = await openMap(page, { ...FIXTURE, live: claudeOn([4, 5, 6]) });
  const book = await around(page, 4);
  await expect.poll(() => rose(page, book)).toBeGreaterThan(30);
  await page.mouse.move(book.x + book.width / 2, book.y + book.height / 2);
  await expect(page.getByRole('tooltip')).toHaveText(
    'Claude Code session 3f2a… is editing book.rs; before that it read mod.rs and payment.rs.',
  );
});

test('the live section lists sessions five at a time, each flying to its newest file', async ({
  page,
}) => {
  const agents = [AGENTS.claude, AGENTS.codex, AGENTS.cursor, AGENTS.other];
  // Seven sessions, ids s0 to s6, each on file 1 + its number (invoice.rs, tax.rs, lib.rs, ...).
  const sessions = Array.from({ length: 7 }, (_, i) =>
    liveSession(`s${i}`, agents[i % 4] ?? AGENTS.other, [[1 + i, 'read']], NOW_MS),
  );
  record = await openMap(page, { ...FIXTURE, live: { version: 1, sessions } });
  const section = page.getByTestId('overview-live');
  await expect(section.getByRole('button', { name: /^Live sessions/ })).toHaveText(
    'Live sessions7',
  );
  await expect(section.getByRole('listitem')).toHaveText([
    'Claude Code s0 reading invoice.rs',
    'Codex s1 reading tax.rs',
    'Cursor s2 reading lib.rs',
    'Aider s3 reading book.rs',
    'Claude Code s4 reading mod.rs',
  ]);
  await section.getByRole('button', { name: 'Show 2 more' }).click();
  await expect(section.getByRole('listitem')).toHaveCount(7);
  await section.getByRole('button', { name: /^Codex session s1 is reading tax.rs/ }).focus();
  await page.keyboard.press('Enter');
  await expect(page.locator('#panel-title')).toHaveText('tax.rs');
  // The sessions end: the section says so and collapses.
  liveChannel(page).push(NO_SESSIONS);
  await expect(section.getByRole('button', { name: /^Live sessions/ })).toHaveText(
    'Live sessions0',
  );
  await expect(section.getByRole('button', { name: /^Live sessions/ })).toHaveAttribute(
    'aria-expanded',
    'false',
  );
});

test('a frame the guard refuses is dropped, and the last state stays drawn', async ({ page }) => {
  const warnings: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'warning') warnings.push(message.text());
  });
  record = await openMap(page, { ...FIXTURE, live: claudeOn([4]) });
  const book = await around(page, 4);
  await expect.poll(() => rose(page, book)).toBeGreaterThan(30);
  const refused = JSON.stringify(claudeOn([6])).replace('"edit"', '"write"');
  liveChannel(page).push(refused);
  await expect.poll(() => warnings.length).toBe(1);
  expect(warnings[0]).toMatch(/^A live frame was dropped \(bad_kind: /);
  expect(await rose(page, book)).toBeGreaterThan(30);
});

test('a lost connection pauses the live view, hides the markers, and it comes back', async ({
  page,
}) => {
  record = await openMap(page, { ...FIXTURE, live: claudeOn([4]) });
  const book = await around(page, 4);
  await expect.poll(() => rose(page, book)).toBeGreaterThan(30);
  const live = liveChannel(page);
  live.setRefusing(true);
  live.closeAll();
  await expect(page.getByTestId('live-paused')).toHaveText('Live view paused, reconnecting.');
  await expect(page.getByTestId('overview-live')).toContainText('Live view paused, reconnecting.');
  await expect.poll(() => rose(page, book)).toBe(0);
  // The page tries again after 1 s, then 2 s; once the daemon answers, the markers return.
  await expect.poll(() => live.openedCount(), { timeout: 5000 }).toBeGreaterThanOrEqual(2);
  live.setRefusing(false);
  await expect(page.getByTestId('live-paused')).toHaveCount(0, { timeout: 10_000 });
  await expect.poll(() => rose(page, book)).toBeGreaterThan(30);
});
