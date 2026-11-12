// The map in a browser, driven with the keyboard: the summary strip, search to a file and its
// selection, the side panel's sections, Escape, Fit and Center, the legend, and the dimming of
// everything outside the hovered or selected file's neighbourhood. Every check reads what the
// page shows: text, focus, or pixels of the canvas. Hover is checked in pixels only: the status
// line announces the selection, never the pointer.
import { expect, test } from '@playwright/test';
import { fitCamera, toScreen } from '../src/map/camera';
import { buildScene } from '../src/map/scene';
import { DARK_COLORS } from '../src/theme/tokens';
import { coldFixture, smallFixture } from './fixtures';
import {
  headerBottom,
  inkIn,
  nextFrames,
  openMap,
  searchAndPick,
  snapshotOf,
  tabTo,
  type PageRecord,
} from './helpers';

const VIEWPORT = { width: 1440, height: 900 };
const PANEL_WIDTH = 380;

let record: PageRecord = { errors: [], foreign: [] };

test.afterEach(() => {
  expect(record.errors, 'console errors').toEqual([]);
  expect(record.foreign, 'requests to other origins').toEqual([]);
});

test.describe('ten-file repository', () => {
  test.beforeEach(async ({ page }) => {
    record = await openMap(page, smallFixture(Date.now()));
  });

  test('the summary strip states the repository in plain sentences', async ({ page }) => {
    await expect(page.getByTestId('summary-indexed')).toHaveText('10 files in 5 folders.');
    await expect(page.getByTestId('summary-explored')).toHaveText(
      'Agents have read 7 of them (70%).',
    );
    await expect(page.getByTestId('summary-pair')).toHaveText(
      'Strongest coupling: book.rs and mod.rs changed together in 12 commits.',
    );
    await expect(page.getByTestId('summary-busiest')).toHaveText(
      'Most worked on in the last 7 days: src/orders/ (52,310 tokens attributed).',
    );
  });

  test('the summary strip keeps to one line until asked for all of it', async ({ page }) => {
    const strip = page.getByRole('region', { name: 'Summary' });
    // One line at 14 px, padded, inside the 32 px control height.
    expect((await strip.boundingBox())?.height).toBeLessThanOrEqual(34);
    await expect(strip.getByTestId('summary-pair')).toBeVisible();
    await tabTo(page, '.summary-toggle');
    await page.keyboard.press('Enter');
    await expect(strip.getByRole('button', { name: 'Show less' })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    expect((await strip.boundingBox())?.height).toBeGreaterThan(60);
  });

  test('search flies to a file, selects it and moves focus into its panel', async ({ page }) => {
    await searchAndPick(page, 'book');
    await expect(page.getByRole('heading', { name: 'book.rs', level: 2 })).toBeFocused();
    // book.rs links above 0.25: mod.rs 0.9, payment.rs 0.8, tests/orders.rs 0.55, ui/app.ts 0.45.
    await expect(page.getByTestId('map-status')).toHaveText(
      'Highlighting src/orders/book.rs and 4 linked files; other files are dimmed.',
    );
  });

  test('the side panel shows every section, and each one empty says so', async ({ page }) => {
    await searchAndPick(page, 'book');
    const panel = page.getByRole('complementary');
    await expect(panel.getByText('src/orders/book.rs', { exact: true })).toBeVisible();
    await expect(panel.locator('dl')).toHaveText(
      /^Lines500Agent reads14Commits21Tokens attributed18,300$/,
    );
    // The list holds only its terms and values; the tokens' meaning is described beside it.
    await expect(panel.locator('dl > :not(div), dl > div > :not(dt, dd)')).toHaveCount(0);
    await expect(panel.locator('dd', { hasText: '18,300' })).toHaveAccessibleDescription(
      'Tokens of agent turns whose last read or edit was this file.',
    );
    await expect(panel.getByTestId('panel-links-co_change')).toHaveText(
      'Changed togethermod.rs12 commitspayment.rs9 commitsorders.rs6 commits',
    );
    await expect(panel.getByTestId('panel-links-used_together')).toHaveText(
      'Used togetherNone learned yet.',
    );
    await expect(panel.getByTestId('panel-links-work_order')).toHaveText(
      'Named in a work orderapp.tsstrength 0.45',
    );
    await expect(panel.getByTestId('panel-facts').locator('li[data-status="suspect"]')).toHaveText(
      'SuspectconventionA cancelled order keeps its book entry.',
    );
    await expect(panel.getByTestId('panel-history')).toContainText(
      'Keep cancelled orders in the book',
    );
    await expect(panel.getByTestId('panel-told').locator('li')).toHaveText([
      /Work order.*then read or edited/,
      /Warning.*not known yet/,
    ]);

    await tabTo(page, '.panel button.link-button[title="src/orders/mod.rs"]');
    await page.keyboard.press('Enter');
    await expect(page.getByRole('heading', { name: 'mod.rs', level: 2 })).toBeFocused();
    await expect(panel.getByTestId('panel-facts')).toHaveText(
      'FactsNo facts are recorded for this file.',
    );
    await expect(panel.getByTestId('panel-history')).toHaveText(
      "HistoryNo commit history read for this file yet; Cortex fills it in as it reads the repository's history.",
    );
    await expect(panel.getByTestId('panel-told')).toHaveText(
      'What Cortex told agentsCortex has not named this file to an agent yet.',
    );
  });

  test('Escape closes the panel and returns focus to the search box', async ({ page }) => {
    await searchAndPick(page, 'payment');
    await expect(page.getByRole('complementary')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.getByRole('complementary')).toHaveCount(0);
    await expect(page.getByRole('searchbox')).toBeFocused();
    await expect(page.getByTestId('map-status')).toHaveText('No file is highlighted.');
  });

  test('Fit shows the whole graph in the visible area; zoom steps by 1.3', async ({ page }) => {
    const zoom = page.locator('output.zoom-value');
    const fitted = await zoom.textContent();
    await page.keyboard.press('+');
    await expect(zoom).not.toHaveText(fitted ?? '');
    const stepped = Number((await zoom.textContent())?.replace('%', ''));
    expect(stepped / Number(fitted?.replace('%', ''))).toBeCloseTo(1.3, 1);
    await page.keyboard.press('z');
    await expect(zoom).toHaveText(fitted ?? '');
    const top = await headerBottom(page);
    const visible = { x: 0, y: top, width: VIEWPORT.width, height: VIEWPORT.height - top };
    const ink = await inkIn(page, visible, { background: DARK_COLORS.background, threshold: 60 });
    expect(ink.box).not.toBeNull();
    // The graph spans most of the visible area on one axis and stays clear of its edges.
    const box = ink.box ?? visible;
    expect(Math.max(box.width / visible.width, box.height / visible.height)).toBeGreaterThan(0.6);
    expect(box.y).toBeGreaterThan(top + 4);
    expect(box.y + box.height).toBeLessThan(VIEWPORT.height - 4);
  });

  test('Center brings the selected file back to the middle of the visible map', async ({
    page,
  }) => {
    await searchAndPick(page, 'book');
    const top = await headerBottom(page);
    const middle = { x: (VIEWPORT.width - PANEL_WIDTH) / 2, y: top + (VIEWPORT.height - top) / 2 };
    const around = { x: middle.x - 25, y: middle.y - 25, width: 50, height: 50 };
    const accent = { background: DARK_COLORS.background, color: DARK_COLORS.accent, threshold: 40 };
    const centred = await inkIn(page, around, accent);
    for (let i = 0; i < 3; i += 1) await page.keyboard.press('ArrowRight');
    await nextFrames(page);
    const panned = await inkIn(page, around, accent);
    await page.keyboard.press('c');
    await nextFrames(page);
    const back = await inkIn(page, around, accent);
    // The selected file is outlined in the accent: there at first, gone after 240 px of panning.
    expect(centred.count).toBeGreaterThan(40);
    expect(panned.count).toBeLessThan(centred.count / 3);
    expect(back.count).toBeGreaterThan(40);
  });

  test('the legend counts links by kind and files by state, and the slider changes them', async ({
    page,
  }) => {
    await tabTo(page, 'button.legend-toggle');
    await page.keyboard.press('Enter');
    const legend = page.getByRole('region', { name: 'Legend' });
    await expect(legend.getByRole('button', { name: 'Legend' })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    // At 0.25: co-change 0.7, 0.9, 0.8, 0.55, 0.3; used together 0.6, 0.4, 0.5; work order 0.35, 0.45.
    await expect(legend.getByTestId('legend-co_change')).toHaveText('Changed together5');
    await expect(legend.getByTestId('legend-used_together')).toHaveText('Used together3');
    await expect(legend.getByTestId('legend-work_order')).toHaveText('Named in a work order2');
    await expect(legend.getByTestId('legend-recent')).toContainText('2');
    await expect(legend.getByTestId('legend-suspect')).toHaveText('Has a suspect fact1');
    await expect(legend.getByTestId('legend-unread')).toHaveText('Never read by an agent3');
    await expect(legend.getByTestId('legend-folders')).toContainText('5');
    await tabTo(page, 'input[type="range"]');
    await expect(page.getByRole('slider')).toBeFocused();
    for (let i = 0; i < 5; i += 1) await page.keyboard.press('ArrowLeft');
    await expect(legend.locator('output')).toHaveText('0.00');
    await expect(legend.getByTestId('legend-work_order')).toHaveText('Named in a work order3');
  });

  test('hovering a file dims every file outside its neighbourhood', async ({ page }) => {
    const scene = buildScene(snapshotOf(smallFixture(Date.now())));
    const top = await headerBottom(page);
    const camera = fitCamera(scene, VIEWPORT, { top, right: 0, bottom: 0, left: 0 });
    const at = (id: number): [number, number] =>
      toScreen(camera, VIEWPORT, scene.layout.fileX[id] ?? 0, scene.layout.fileY[id] ?? 0);
    // src/lib.rs (3) is no neighbour of payment.rs (6); its outline is the plain file grey.
    const [libX, libY] = at(3);
    const libBox = { x: libX - 16, y: libY - 16, width: 32, height: 32 };
    const ink = { background: DARK_COLORS.background, threshold: 60 };
    const before = await inkIn(page, libBox, ink);
    const [x, y] = at(6);
    await page.mouse.move(x, y);
    expect(before.count).toBeGreaterThan(20);
    await expect
      .poll(async () => (await inkIn(page, libBox, ink)).count)
      .toBeLessThan(before.count / 4);
    // Hover is not announced; only a selection is.
    await expect(page.getByTestId('map-status')).toHaveText('No file is highlighted.');
  });
});

test.describe('cold repository', () => {
  test('says nothing is learned yet and still draws the structure', async ({ page }) => {
    record = await openMap(page, coldFixture());
    await expect(page.getByTestId('summary-indexed')).toHaveText('6 files in 3 folders.');
    await expect(page.getByTestId('summary-cold')).toHaveText(
      'Cortex has not learned anything here yet, so the map shows the structure alone.',
    );
    const top = await headerBottom(page);
    const visible = { x: 0, y: top, width: VIEWPORT.width, height: VIEWPORT.height - top };
    const ink = await inkIn(page, visible, { background: DARK_COLORS.background, threshold: 60 });
    expect(ink.count).toBeGreaterThan(200);
  });
});
