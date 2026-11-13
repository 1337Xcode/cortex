// The page under the daemon's security headers, and its failed state when rendering throws. The
// map is served with the content security policy it runs under for real; drawing it must log no
// error (a refused worker, script, style or connection logs one) and ask no other origin for
// anything, its brand and language icons among it. A render that throws shows the failed state
// with its cause, never a blank page.
import { expect, test } from '@playwright/test';
import { smallFixture, threeSessions } from './fixtures';
import { openMap, searchAndPick } from './helpers';
import { MAP_URL, SECURITY_HEADERS } from './serve';

test("the map draws under the daemon's security headers with no error", async ({ page }) => {
  const nowMs = Date.now();
  const record = await openMap(page, { ...smallFixture(nowMs), live: threeSessions(nowMs) });
  const response = await page.goto(MAP_URL);
  const headers = response?.headers() ?? {};
  for (const [name, value] of Object.entries(SECURITY_HEADERS)) expect(headers[name]).toBe(value);
  await expect(page.getByTestId('summary-indexed')).toHaveText('10 files in 5 folders.');
  // A file's panel loads its detail: one more request under the policy.
  await searchAndPick(page, 'book');
  await page.getByRole('button', { name: /^Commits behind its lines/ }).click();
  await expect(page.getByTestId('panel-history')).toContainText('Keep cancelled orders');
  // The icons are drawn inline from the page's own script: agents' in the live sessions and the
  // legend, files' in the overview and the panel.
  await page.getByRole('button', { name: 'Legend' }).click();
  await expect(
    page.getByTestId('overview-live').locator('[data-icon="claude-code"] svg'),
  ).toBeVisible();
  await expect(page.locator('.legend [data-icon="codex-openai"] svg')).toBeVisible();
  await expect(page.locator('.file-panel [data-icon="rust"] svg').first()).toBeVisible();
  expect(record.errors, 'console errors').toEqual([]);
  expect(record.foreign, 'requests to other origins').toEqual([]);
});

test('a render that throws shows the failed state, not a blank page', async ({ page }) => {
  // The history and told sections read a date's UTC year, and both open only when asked; make
  // that throw, then open the history.
  await page.addInitScript(() => {
    Date.prototype.getUTCFullYear = () => {
      throw new Error('the year cannot be read');
    };
  });
  const record = await openMap(page, smallFixture(Date.now()));
  await searchAndPick(page, 'book');
  await page.getByRole('button', { name: /^Commits behind its lines/ }).click();
  await expect(page.getByRole('alert')).toHaveText(
    'The map stopped with an error (render): the year cannot be read',
  );
  await expect(page.getByRole('heading', { name: 'Cortex map', level: 1 })).toBeVisible();
  // React reports the error it caught; nothing else may have gone wrong.
  expect(record.errors.length).toBeGreaterThan(0);
  expect(record.errors.filter((error) => !error.includes('the year cannot be read'))).toEqual([]);
  expect(record.foreign).toEqual([]);
});
