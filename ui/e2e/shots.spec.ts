// Screenshots of every state of the map at 1440x900 and 390x844 in both themes, written to the
// folder named by CORTEX_SHOTS_DIR for a person to look at: first open, the overview, a search, a
// selected file with every section open, the legend, nothing found, the cold, empty, loading and
// failed states, each view with its legend, three live sessions, the paused live view, the
// receipts cold, at 29 tasks a side, at 30 a side and failed, and 100,000 files with and without
// twenty sessions. Skipped when that variable is unset.
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import {
  coldFixture,
  largeFixture,
  manySessions,
  READING_HEALTH,
  shortReceipts,
  smallFixture,
  threeSessions,
  type Fixture,
} from './fixtures';
import { nextFrames, openMap, tabTo } from './helpers';
import { liveChannel, MAP_URL, serveFixture } from './serve';

const SHOTS_DIR = process.env['CORTEX_SHOTS_DIR'] ?? '';
const SIZES = [
  { name: '1440x900', width: 1440, height: 900 },
  { name: '390x844', width: 390, height: 844 },
] as const;
const SCHEMES = ['dark', 'light'] as const;

test.skip(SHOTS_DIR === '', 'set CORTEX_SHOTS_DIR to write screenshots');

async function shoot(page: Page, name: string): Promise<void> {
  await nextFrames(page);
  await page.screenshot({ path: join(SHOTS_DIR, `${name}.png`) });
}

async function searchFor(page: Page, text: string): Promise<void> {
  await page.keyboard.press('/');
  await page.keyboard.type(text);
  await expect(page.locator('.search-popover')).toBeVisible();
  await expect(page.locator('.search-popover')).not.toContainText('Searching.');
}

/** Opens every closed section of the file panel. */
async function openSections(page: Page): Promise<void> {
  const closed = page.locator('.file-panel .section-toggle[aria-expanded="false"]');
  while ((await closed.count()) > 0) await closed.first().click();
}

for (const size of SIZES) {
  for (const scheme of SCHEMES) {
    const prefix = `${size.name}-${scheme}`;
    const isNarrow = size.width < 720;

    test(`${prefix}: ten-file repository`, async ({ page }) => {
      await page.setViewportSize(size);
      const record = await openMap(page, smallFixture(Date.now()), scheme, true);
      await shoot(page, `${prefix}-01-first-open`);
      await page.getByRole('button', { name: 'Got it' }).click();
      if (isNarrow) {
        await page.getByRole('button', { name: 'Overview' }).click();
        await shoot(page, `${prefix}-02-overview`);
        await page.getByRole('button', { name: 'Close the overview' }).click();
      } else {
        await shoot(page, `${prefix}-02-overview`);
      }
      await searchFor(page, 'order');
      await shoot(page, `${prefix}-03-search`);
      await page.keyboard.press('Enter');
      await expect(page.getByRole('complementary', { name: 'book.rs' })).toBeVisible();
      await shoot(page, `${prefix}-04-selected`);
      await openSections(page);
      await shoot(page, `${prefix}-05-selected-open`);
      await page.keyboard.press('Escape');
      await page.getByRole('button', { name: 'Legend' }).click();
      await shoot(page, `${prefix}-06-legend`);
      await page.getByRole('button', { name: 'Legend' }).click();
      await searchFor(page, 'zzz');
      await shoot(page, `${prefix}-07-nothing-found`);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: cold, empty, loading and failed`, async ({ page }) => {
      await page.setViewportSize(size);
      const record = await openMap(page, coldFixture(), scheme);
      await shoot(page, `${prefix}-08-cold`);
      const empty: Fixture = { ...coldFixture(), graph: { version: 1, files: [], links: [] } };
      await page.unrouteAll();
      await serveFixture(page, { ...empty, health: READING_HEALTH });
      await page.goto(MAP_URL);
      await expect(page.getByText('Cortex is still reading this repository.')).toBeVisible();
      await shoot(page, `${prefix}-09-empty-reading`);
      await page.unrouteAll();
      await serveFixture(page, empty);
      await page.goto(MAP_URL);
      await expect(page.getByText('This repository has no files to map.')).toBeVisible();
      await shoot(page, `${prefix}-09-empty-finished`);
      let release = (): void => undefined;
      const held = new Promise<void>((resolve) => {
        release = resolve;
      });
      await page.route('**/api/graph', async (route) => {
        await held;
        await route.fulfill({ status: 500, body: '' });
      });
      await page.goto(MAP_URL);
      await expect(page.getByText('Loading the map.')).toBeVisible();
      await shoot(page, `${prefix}-10-loading`);
      release();
      await expect(page.getByRole('alert')).toBeVisible();
      await shoot(page, `${prefix}-11-failed`);
      expect(record.errors.filter((error) => !error.includes('500'))).toEqual([]);
    });

    test(`${prefix}: views and live sessions`, async ({ page }) => {
      await page.setViewportSize(size);
      const nowMs = Date.now();
      const fixture: Fixture = { ...smallFixture(nowMs), live: threeSessions(nowMs) };
      const record = await openMap(page, fixture, scheme);
      if (!isNarrow) {
        await expect(page.getByTestId('overview-live').getByRole('listitem')).toHaveCount(3);
      }
      await shoot(page, `${prefix}-15-live`);
      for (const [key, name] of [
        ['2', 'fog'],
        ['3', 'heat'],
        ['4', 'coupling'],
        ['5', 'recent'],
      ] as const) {
        await page.keyboard.press(key);
        if (key === '2') await page.keyboard.press('l');
        await shoot(page, `${prefix}-16-view-${name}`);
      }
      await page.keyboard.press('l');
      await page.keyboard.press('1');
      await page.keyboard.press('/');
      await page.keyboard.type('book');
      await expect(page.locator('.search-popover')).not.toContainText('Searching.');
      await page.keyboard.press('Enter');
      await expect(page.locator('.file-panel')).toBeVisible();
      await shoot(page, `${prefix}-17-live-selected`);
      await page.keyboard.press('Escape');
      const live = liveChannel(page);
      live.setRefusing(true);
      live.closeAll();
      await expect(page.getByTestId('live-paused')).toBeVisible();
      await shoot(page, `${prefix}-18-live-paused`);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: the receipts in each state`, async ({ page }) => {
      await page.setViewportSize(size);
      const nowMs = Date.now();
      const states: [string, Fixture][] = [
        ['cold', coldFixture()],
        ['short', { ...smallFixture(nowMs), receipts: shortReceipts() }],
        ['compared', smallFixture(nowMs)],
      ];
      for (const [name, fixture] of states) {
        await page.unrouteAll();
        const record = await openMap(page, fixture, scheme);
        if (isNarrow) await page.getByRole('button', { name: 'Overview' }).click();
        await page.getByRole('button', { name: /^Receipts/ }).click();
        const section = page.getByTestId('overview-receipts');
        await expect(section.getByTestId('receipts-headline')).toBeVisible();
        await shoot(page, `${prefix}-21-receipts-${name}`);
        // The chart, focused and explaining its first task; then the end of the section.
        const chart = section.getByTestId('receipts-chart');
        if ((await chart.count()) > 0) {
          await tabTo(page, '.chart-input');
          await page.keyboard.press('Home');
          await chart.scrollIntoViewIfNeeded();
          await shoot(page, `${prefix}-22-receipts-${name}-chart`);
        }
        await section.getByTestId('receipts-prices').scrollIntoViewIfNeeded();
        await shoot(page, `${prefix}-23-receipts-${name}-end`);
        expect(record.errors).toEqual([]);
      }
      await page.unrouteAll();
      const record = await openMap(page, smallFixture(nowMs), scheme);
      await page.route('**/api/receipts', (route) => route.fulfill({ status: 500, body: '' }));
      if (isNarrow) await page.getByRole('button', { name: 'Overview' }).click();
      await page.getByRole('button', { name: /^Receipts/ }).click();
      await expect(page.getByTestId('receipts-error')).toBeVisible();
      await shoot(page, `${prefix}-24-receipts-failed`);
      expect(record.errors.filter((error) => !error.includes('500'))).toEqual([]);
    });

    test(`${prefix}: 100,000 files with twenty sessions, in Heat`, async ({ page }) => {
      test.setTimeout(240_000);
      await page.setViewportSize(size);
      const large: Fixture = largeFixture(100_000);
      const record = await openMap(
        page,
        { ...large, live: manySessions(20, 100_000, Date.now()) },
        scheme,
      );
      await shoot(page, `${prefix}-19-large-live`);
      await page.keyboard.press('3');
      await shoot(page, `${prefix}-20-large-heat`);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: 100,000 files`, async ({ page }) => {
      test.setTimeout(240_000);
      await page.setViewportSize(size);
      const large: Fixture = largeFixture(100_000);
      const record = await openMap(page, large, scheme);
      await shoot(page, `${prefix}-12-large-overview`);
      await searchFor(page, 'pkg3/mod4');
      await page.keyboard.press('Enter');
      await expect(page.locator('.file-panel')).toBeVisible();
      await shoot(page, `${prefix}-13-large-files`);
      await page.keyboard.press('Escape');
      await page.mouse.move(1, size.height / 2);
      await shoot(page, `${prefix}-14-large-dense`);
      expect(record.errors).toEqual([]);
    });
  }
}
