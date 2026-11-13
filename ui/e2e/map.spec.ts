// The map in a browser, driven with the keyboard: the one-line summary strip and the overview
// panel, search to a file and its selection, the side panel's sections, Escape, Fit and Center,
// the legend, the first-open sentence, and the dimming of everything outside the hovered or
// selected file's neighbourhood. Every check reads what the page shows: text, focus, or pixels of
// the canvas. Hover is checked in pixels and in its explanation; the status line announces the
// selection, never the pointer.
import { expect, test } from '@playwright/test';
import { fitCamera, toScreen } from '../src/map/camera';
import { buildScene } from '../src/map/scene';
import { DARK_COLORS, SIZE } from '../src/theme/tokens';
import { coldFixture, fileDetail, READING_HEALTH, smallFixture } from './fixtures';
import { MAP_URL, serveFixture } from './serve';
import {
  chromeInsets,
  filePanel,
  headerBottom,
  inkIn,
  nextFrames,
  openMap,
  searchAndPick,
  snapshotOf,
  type PageRecord,
} from './helpers';

const VIEWPORT = { width: 1440, height: 900 };

let record: PageRecord = { errors: [], foreign: [] };

test.afterEach(() => {
  expect(record.errors, 'console errors').toEqual([]);
  expect(record.foreign, 'requests to other origins').toEqual([]);
});

test.describe('ten-file repository', () => {
  test.beforeEach(async ({ page }) => {
    record = await openMap(page, smallFixture(Date.now()));
  });

  test('the overview states the repository in plain sentences', async ({ page }) => {
    await expect(page.getByTestId('summary-indexed')).toHaveText('10 files in 5 folders.');
    await expect(page.getByTestId('summary-explored')).toHaveText(
      'Agents have read 7 of the 10 files (70%).',
    );
    await expect(page.getByTestId('summary-pair')).toHaveText(
      'Strongest coupling: book.rs and mod.rs changed together in 12 commits.',
    );
    await expect(page.getByTestId('summary-busiest')).toHaveText(
      'Most worked on in the last 7 days: src/orders/, whose own files took 52,310 tokens attributed.',
    );
  });

  test('the summary strip stays one line of whole sentences', async ({ page }) => {
    const strip = page.getByRole('region', { name: 'Summary' });
    // One control high: 32 px, whatever it holds.
    expect((await strip.boundingBox())?.height).toBe(SIZE.control);
    // The most useful sentence leads, whole; nothing is cut inside a word.
    await expect(strip.locator('[data-strip]').first()).toHaveText(
      'Strongest coupling: book.rs and mod.rs changed together in 12 commits.',
    );
    await expect(strip).not.toContainText('…');
    await expect(strip.getByRole('button', { name: /Show all/ })).toHaveCount(0);
  });

  test('the overview lists the strongest pairs, the busiest files and what needs attention', async ({
    page,
  }) => {
    const overview = page.getByRole('complementary', { name: 'Overview' });
    // Co-change pairs by weight, as the summary ranks them: book+mod 0.9 (12 commits),
    // book+payment 0.8 (9), invoice+tax 0.7 (5), book+orders 0.55 (6), payment+orders 0.3 (3).
    // Rows are read as drawn: a row's icon only repeats its name, so its title is not text.
    const pairs = overview.getByTestId('overview-pairs');
    await expect(pairs.getByRole('listitem')).toHaveText(
      [
        'book.rs and mod.rs 12 commits',
        'book.rs and payment.rs 9 commits',
        'invoice.rs and tax.rs 5 commits',
        'book.rs and orders.rs 6 commits',
        'payment.rs and orders.rs 3 commits',
      ],
      { useInnerText: true },
    );
    // Both files of every pair are Rust, so every pair's row carries the Rust icon.
    await expect(pairs.locator('[data-icon="rust"] svg')).toHaveCount(5);
    // Seven files have tokens; five show, then "Show 2 more".
    const busiest = overview.getByTestId('overview-busiest');
    await expect(busiest.getByRole('listitem')).toHaveCount(5);
    await expect(busiest.getByRole('listitem').first()).toHaveText('payment.rs 29,910 tokens', {
      useInnerText: true,
    });
    await busiest.getByRole('button', { name: 'Show 2 more' }).click();
    await expect(busiest.getByRole('listitem')).toHaveCount(7);
    // The seven with tokens: invoice, tax, lib, book, mod and payment in Rust, and ui/app.ts.
    await expect(busiest.locator('[data-icon="rust"] svg')).toHaveCount(6);
    await expect(busiest.locator('[data-icon="typescript"] svg')).toHaveCount(1);
    await expect(overview.getByTestId('overview-attention').getByRole('listitem')).toHaveText(
      ['book.rs 1 suspect fact'],
      { useInnerText: true },
    );
    // A row is a button that flies to its file and selects it.
    await overview
      .getByRole('button', { name: /^src\/orders\/book.rs has 1 suspect fact/ })
      .click();
    await expect(page.locator('#panel-title')).toHaveText('book.rs');
  });

  test('search flies to a file, selects it and moves focus into its panel', async ({ page }) => {
    await searchAndPick(page, 'book');
    await expect(page.locator('#panel-title')).toBeFocused();
    // book.rs links above 0.25: mod.rs 0.9, payment.rs 0.8, tests/orders.rs 0.55, ui/app.ts 0.45.
    await expect(page.getByTestId('map-status')).toHaveText(
      'Highlighting src/orders/book.rs and 4 linked files; other files are dimmed.',
    );
  });

  test('the side panel shows every section with its count, and each one empty says so', async ({
    page,
  }) => {
    await searchAndPick(page, 'book');
    const panel = filePanel(page);
    await expect(panel.getByText('src/orders/book.rs', { exact: true })).toBeVisible();
    await expect(
      panel.getByRole('navigation', { name: 'Folders of the selected file' }),
    ).toHaveText(
      // The slashes between the folders are drawn by the stylesheet, not written in the page.
      'Repositorysrcorders',
    );
    await expect(panel.locator('dl')).toHaveText(
      /^Lines500Agent reads14Commits21Tokens attributed18,300$/,
    );
    // The list holds only its terms and values; the tokens' meaning is described beside it.
    await expect(panel.locator('dl > :not(div), dl > div > :not(dt, dd)')).toHaveCount(0);
    await expect(panel.locator('dd', { hasText: '18,300' })).toHaveAccessibleDescription(
      'Tokens of agent turns whose last read or edit was this file: the input and output tokens of those responses, cache reads included.',
    );
    // Linked files and facts are open; the commits behind its lines, what Cortex told agents and
    // its symbols are closed.
    const toggle = (name: RegExp) => panel.getByRole('button', { name });
    await expect(toggle(/^Linked files/)).toHaveAttribute('aria-expanded', 'true');
    await expect(toggle(/^Facts/)).toHaveAttribute('aria-expanded', 'true');
    await expect(toggle(/^Commits behind its lines/)).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle(/^What Cortex told agents/)).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle(/^Symbols/)).toHaveAttribute('aria-expanded', 'false');
    await expect(toggle(/^Linked files/)).toHaveText('Linked files4');
    await expect(panel.getByTestId('panel-links-co_change')).toHaveText(
      'Changed together mod.rs 12 commits payment.rs 9 commits orders.rs 6 commits',
      { useInnerText: true },
    );
    await expect(panel.getByTestId('panel-links-used_together')).toHaveText(
      'Used togetherNone learned yet.',
    );
    await expect(panel.getByTestId('panel-links-work_order')).toHaveText(
      'Named in a work order app.ts 3 work orders',
      { useInnerText: true },
    );
    await expect(panel.locator('.panel-title-row [data-icon="rust"] svg')).toBeVisible();
    await expect(
      panel.getByTestId('panel-links-work_order').locator('[data-icon="typescript"] svg'),
    ).toBeVisible();
    await expect(panel.getByTestId('panel-facts').locator('li[data-status="suspect"]')).toHaveText(
      'SuspectconventionA cancelled order keeps its book entry.Confirm againRetire',
    );
    await toggle(/^Commits behind its lines/).click();
    await expect(panel.getByTestId('panel-history')).toContainText(
      'The 2 commits behind its current lines, newest first.',
    );
    await expect(panel.getByTestId('panel-history')).toContainText(
      'Keep cancelled orders in the book',
    );
    await toggle(/^What Cortex told agents/).click();
    await expect(panel.getByTestId('panel-told').getByRole('listitem')).toHaveText([
      /Work order.*then read or edited/,
      /Warning.*not known yet/,
    ]);

    await panel
      .getByRole('button', { name: /^book.rs changed with mod.rs in 12 of 21 commits/ })
      .focus();
    await page.keyboard.press('Enter');
    await expect(page.locator('#panel-title')).toHaveText('mod.rs');
    await expect(page.locator('#panel-title')).toBeFocused();
    await expect(panel.getByTestId('panel-facts')).toHaveText(
      'Facts0No facts are recorded for this file.',
    );
    // A section opened for one file stays open for the next.
    await expect(toggle(/^Commits behind its lines/)).toHaveAttribute('aria-expanded', 'true');
    await expect(panel.getByTestId('panel-history')).toHaveText(
      "Commits behind its lines0No commit is stored for this file's current lines yet. Cortex reads them when a prompt or its upkeep asks about the file, and lists the 10 newest.",
    );
  });

  test('the panel opens its file in an editor and says what its commits count', async ({
    page,
  }) => {
    await searchAndPick(page, 'book');
    const panel = filePanel(page);
    await expect(panel.getByRole('link', { name: 'Open in editor' })).toHaveAttribute(
      'href',
      'vscode://file/C:/repo/src/orders/book.rs:1',
    );
    const commits = panel.locator('dd', { hasText: '21' });
    const meaning =
      'Commits Cortex read that changed this file: the last 5,000 commits within 365 days, leaving out commits that changed more than 30 files.';
    await expect(commits).toHaveAccessibleDescription(meaning);
    await commits.locator('.number-explained').focus();
    await expect(page.getByRole('tooltip')).toHaveText(meaning);
  });

  test('Escape closes the panel and returns focus to the search box', async ({ page }) => {
    await searchAndPick(page, 'payment');
    await expect(filePanel(page)).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(filePanel(page)).toHaveCount(0);
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
    const insets = await chromeInsets(page, false);
    const visible = {
      x: insets.left,
      y: insets.top,
      width: VIEWPORT.width - insets.left,
      height: VIEWPORT.height - insets.top,
    };
    const ink = await inkIn(page, visible, { background: DARK_COLORS.background, threshold: 60 });
    expect(ink.box).not.toBeNull();
    // The graph spans most of the visible area on one axis and stays clear of its edges.
    const box = ink.box ?? visible;
    expect(Math.max(box.width / visible.width, box.height / visible.height)).toBeGreaterThan(0.6);
    expect(box.y).toBeGreaterThan(insets.top + 4);
    expect(box.y + box.height).toBeLessThan(VIEWPORT.height - 4);
  });

  test('Center brings the selected file back to the middle of the visible map', async ({
    page,
  }) => {
    await searchAndPick(page, 'book');
    const insets = await chromeInsets(page, true);
    const middle = {
      x: insets.left + (VIEWPORT.width - insets.left - insets.right) / 2,
      y: insets.top + (VIEWPORT.height - insets.top) / 2,
    };
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

  test('the legend counts each shape and says what it means; the slider changes the counts', async ({
    page,
  }) => {
    await page.keyboard.press('l');
    const legend = page.getByRole('region', { name: 'Legend' });
    await expect(legend.getByRole('button', { name: 'Legend' })).toHaveAttribute(
      'aria-expanded',
      'true',
    );
    const count = (id: string) => legend.getByTestId(id).locator('.legend-count');
    const meaning = (id: string) => legend.getByTestId(id).locator('.legend-meaning');
    // At 0.25: co-change 0.7, 0.9, 0.8, 0.55, 0.3; used together 0.6, 0.4, 0.5; work order 0.35, 0.45.
    await expect(count('legend-co_change')).toHaveText('5');
    await expect(count('legend-used_together')).toHaveText('3');
    await expect(count('legend-work_order')).toHaveText('2');
    await expect(meaning('legend-co_change')).toHaveText(
      "Solid: the two files changed in the same commits. Strength: the larger share of either file's commits that changed both, recent commits counting more.",
    );
    // payment.rs 2 h and app.ts 5 h ago are recent; view.ts (72 h) is not.
    await expect(count('legend-recent')).toHaveText('2');
    await expect(meaning('legend-recent')).toHaveText('A filled centre, in green.');
    await expect(count('legend-suspect')).toHaveText('1');
    await expect(meaning('legend-suspect')).toHaveText('A second ring, in amber.');
    // Never read, and neither recent nor suspect: README, tests/orders.rs, ui/view.ts.
    await expect(count('legend-unread')).toHaveText('3');
    await expect(count('legend-folders')).toHaveText('5');
    await legend.getByRole('slider').focus();
    for (let i = 0; i < 5; i += 1) await page.keyboard.press('ArrowLeft');
    await expect(legend.locator('output')).toHaveText('0.00');
    await expect(count('legend-work_order')).toHaveText('3');
  });

  test('hovering a file dims every file outside its neighbourhood and explains the file', async ({
    page,
  }) => {
    const scene = buildScene(snapshotOf(smallFixture(Date.now())));
    const insets = await chromeInsets(page, false);
    const camera = fitCamera(scene, VIEWPORT, insets);
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
    // payment.rs: 220 lines, 9 reads, 13 commits, edited 2 hours ago.
    await expect(page.getByRole('tooltip')).toHaveText(
      'payment.rs: 220 lines, read 9 times by agents, changed in 13 commits. Read or edited in the last 24 hours.',
    );
    // Hover is not announced; only a selection is.
    await expect(page.getByTestId('map-status')).toHaveText('No file is highlighted.');
  });
});

test.describe('first open', () => {
  test('says what the picture is once, and the legend says it again', async ({ page }) => {
    record = await openMap(page, smallFixture(Date.now()), 'dark', true);
    const intro = page.getByRole('region', { name: 'What this map shows' });
    await expect(intro).toHaveText(
      'Folders are squares, files are circles on their rings; lines join files that changed in the same commits, that agents used in one task, or that Cortex named in one work order.Got it',
    );
    await intro.getByRole('button', { name: 'Got it' }).click();
    await expect(intro).toHaveCount(0);
    await page.reload();
    await expect(page.getByTestId('map-status')).toBeAttached();
    await expect(intro).toHaveCount(0);
    await page.keyboard.press('l');
    await page.getByRole('button', { name: 'What does this picture show?' }).click();
    await expect(intro).toBeVisible();
  });
});

test.describe('empty repository', () => {
  const empty = { ...coldFixture(), graph: { version: 1 as const, files: [], links: [] } };

  test('says Cortex is still reading it, and shows the map once it has', async ({ page }) => {
    const errors: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    const foreign = await serveFixture(page, { ...empty, health: READING_HEALTH });
    await page.goto(MAP_URL);
    await expect(page.getByRole('status')).toHaveText('Cortex is still reading this repository.');
    await expect(
      page.getByText(/^The map appears here by itself once it has finished/),
    ).toBeVisible();
    // The reading ends: the health says so, the page asks for the map again by itself (it asks
    // every 5 seconds), and shows it with a summary of the files it now shows.
    await serveFixture(page, smallFixture(Date.now()));
    await expect(page.getByTestId('map-status')).toBeAttached({ timeout: 15_000 });
    await expect(page.getByTestId('summary-indexed')).toHaveText('10 files in 5 folders.');
    record = { errors, foreign };
  });

  test('says plainly that there is nothing to map once Cortex has finished reading', async ({
    page,
  }) => {
    const errors: string[] = [];
    page.on('console', (message) => {
      if (message.type() === 'error') errors.push(message.text());
    });
    const foreign = await serveFixture(page, empty);
    await page.goto(MAP_URL);
    await expect(page.getByRole('status')).toHaveText('This repository has no files to map.');
    await expect(
      page.getByText(/^Cortex has finished reading it and found no files/),
    ).toBeVisible();
    await expect(page.getByText(/still reading/)).toHaveCount(0);
    record = { errors, foreign };
  });
});

test.describe('the editor link and a file no commit Cortex read changed', () => {
  test('a link the page may not follow is not shown, and zero commits is not "never"', async ({
    page,
  }) => {
    const cold = coldFixture();
    const detail = fileDetail(1, 'src/cli.rs', { editor_url: 'javascript:alert(1)' });
    record = await openMap(page, { ...cold, details: new Map([[1, detail]]) });
    await searchAndPick(page, 'cli.rs');
    const panel = filePanel(page);
    await expect(panel.getByTestId('panel-facts')).toContainText('No facts are recorded');
    await expect(panel.getByRole('link')).toHaveCount(0);
    await expect(panel.locator('dd', { hasText: /^0$/ }).first()).toBeVisible();
    await expect(panel.locator('dd').nth(2)).toHaveAccessibleDescription(
      /^Commits Cortex read that changed this file: .*\. None of them changed it; older changes, and changes in larger commits, are not counted\.$/,
    );
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
