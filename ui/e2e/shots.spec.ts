// Screenshots of every state of the map at 1440x900 and 390x844 in both themes, written to the
// folder named by CORTEX_SHOTS_DIR for a person to look at. Skipped when that variable is unset.
import { join } from 'node:path';
import { expect, test, type Page } from '@playwright/test';
import { coldFixture, largeFixture, smallFixture, type Fixture } from './fixtures';
import { nextFrames, openMap } from './helpers';
import { MAP_URL } from './serve';

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
  await expect(
    page.getByRole('list', { name: 'Search results' }).getByRole('button').first(),
  ).toBeVisible();
}

for (const size of SIZES) {
  for (const scheme of SCHEMES) {
    const prefix = `${size.name}-${scheme}`;

    test(`${prefix}: ten-file repository`, async ({ page }) => {
      await page.setViewportSize(size);
      const record = await openMap(page, smallFixture(Date.now()), scheme);
      await shoot(page, `${prefix}-1-overview`);
      await searchFor(page, 'order');
      await shoot(page, `${prefix}-2-search`);
      await page.keyboard.press('Enter');
      await expect(page.getByRole('complementary')).toBeVisible();
      await shoot(page, `${prefix}-3-selected`);
      await page.keyboard.press('Escape');
      await page.locator('body').click({ position: { x: 1, y: size.height - 1 } });
      await page.getByRole('button', { name: 'Legend' }).focus();
      await page.keyboard.press('Enter');
      await shoot(page, `${prefix}-4-legend`);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: cold and failed`, async ({ page }) => {
      await page.setViewportSize(size);
      const record = await openMap(page, coldFixture(), scheme);
      await shoot(page, `${prefix}-5-cold`);
      await page.route('**/api/graph', (route) => route.fulfill({ status: 500, body: '' }));
      await page.goto(MAP_URL);
      await expect(page.getByRole('alert')).toBeVisible();
      await shoot(page, `${prefix}-6-failed`);
      expect(record.errors.filter((error) => !error.includes('500'))).toEqual([]);
    });

    test(`${prefix}: 100,000 files`, async ({ page }) => {
      test.setTimeout(240_000);
      await page.setViewportSize(size);
      const large: Fixture = largeFixture(100_000);
      const record = await openMap(page, large, scheme);
      await shoot(page, `${prefix}-7-large-overview`);
      await searchFor(page, 'pkg3/mod4');
      await page.keyboard.press('Enter');
      await expect(page.getByRole('complementary')).toBeVisible();
      await shoot(page, `${prefix}-8-large-files`);
      await page.keyboard.press('Escape');
      await page.mouse.move(1, size.height / 2);
      await shoot(page, `${prefix}-9-large-dense`);
      expect(record.errors).toEqual([]);
    });
  }
}
