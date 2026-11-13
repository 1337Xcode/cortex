// The receipts panel against the daemon's contract, in its three states and its failure: a cold
// store says there is not enough data yet and that no session started; 29 tasks a side says so with
// its counts and states no share, sign or saving anywhere; 30 a side shows the comparison with its
// intervals and sample sizes. The chart's sentence is read with the keyboard, and the receipts are
// asked for only once the section opens. Every expected line is worked from the fixtures by hand
// (see `receiptSessions` and the receipt builders in fixtures.ts).
import { expect, test, type Locator, type Page } from '@playwright/test';
import {
  coldFixture,
  longReceipts,
  shortReceipts,
  shortReceiptsWithComparison,
  smallFixture,
  type Fixture,
} from './fixtures';
import { openMap, tabTo } from './helpers';

/** The receipts section of the overview. */
function receiptsSection(page: Page): Locator {
  return page.getByTestId('overview-receipts');
}

/** Opens the receipts section with the keyboard and waits for its headline. */
async function openReceipts(page: Page): Promise<Locator> {
  await tabTo(page, '[data-testid="overview-receipts"] .section-toggle');
  await page.keyboard.press('Enter');
  const section = receiptsSection(page);
  await expect(section.getByTestId('receipts-headline')).toBeVisible();
  return section;
}

/** The section's text says no share, no saving and nothing saved. */
async function expectNoClaim(section: Locator): Promise<void> {
  const text = (await section.textContent()) ?? '';
  expect(text).not.toMatch(/%/);
  expect(text).not.toMatch(/sav(e|ed|es|ing)\b/i);
  expect(text).not.toMatch(/interval|difference|with Cortex,/);
}

/** The ten-file repository with receipts of 29 tasks a side. */
function shortFixture(nowMs: number): Fixture {
  return { ...smallFixture(nowMs), receipts: shortReceipts() };
}

test('a cold store has not enough data yet, no session and no prices', async ({ page }) => {
  const record = await openMap(page, coldFixture());
  const section = await openReceipts(page);
  await expect(section.getByTestId('receipts-headline').locator('.receipts-lead')).toHaveText(
    'Not enough data yet: 0 of 30 held-out and 0 of 30 treated tasks so far.',
  );
  await expect(section.getByTestId('receipts-precision')).toHaveText(
    'Injection precision: no work order given is labelled yet.',
  );
  await expect(section.getByTestId('receipts-empty')).toHaveText(
    "No session started in the last 7 days. Receipts fill in as agents work in this repository with Cortex's hooks installed (cortex init sets them up).",
  );
  await expect(section.getByTestId('receipts-prices')).toHaveText(
    'Prices: none, so no cost is estimated (there is no home folder to keep a price table in).',
  );
  await expect(section.getByTestId('receipts-chart')).toHaveCount(0);
  await expectNoClaim(section);
  expect(record.errors).toEqual([]);
});

test('29 tasks a side say not enough data yet and state no comparison', async ({ page }) => {
  const record = await openMap(page, shortFixture(Date.now()));
  const section = await openReceipts(page);
  await expect(section.getByTestId('receipts-headline').locator('.receipts-lead')).toHaveText(
    'Not enough data yet: 29 of 30 held-out and 29 of 30 treated tasks so far.',
  );
  await expect(section.getByTestId('receipts-measure')).toHaveCount(0);
  // 7 of 9 is 0.777..., 1 of 4 is 0.25.
  await expect(section.getByTestId('receipts-precision')).toHaveText(
    'Injection precision: 0.78, 7 of 9 work orders given and labelled were used.',
  );
  await expect(section.getByTestId('receipts-warnings')).toHaveText(
    'Warning precision: 0.25, 1 of 4 files warnings named and marked were then read or edited.',
  );
  // Newest first: the running Claude Code session, Codex, then the first.
  await expect(section.getByTestId('receipts-overhead')).toHaveText([
    '600 tokens injected by Cortex (primer: 300 tokens), against 32,000 input tokens in total.',
    '120 tokens injected by Cortex.',
    '750 tokens injected by Cortex (primer: 400 tokens), against 49,300 input tokens in total.',
  ]);
  await expect(section.getByTestId('receipts-prices')).toHaveText(
    'Prices: as of 2026-11-01, from the table Cortex ships, as you have no price file. Every cost is an estimate.',
  );
  await expectNoClaim(section);
  expect(record.errors).toEqual([]);
});

test('the chart explains each task in a sentence, read with the keyboard', async ({ page }) => {
  const record = await openMap(page, shortFixture(Date.now()));
  const section = await openReceipts(page);
  const readout = section.getByTestId('receipts-readout');
  // The newest of the four tasks drawn: task 106, 2 turns of $0.0025 each.
  await expect(readout).toHaveText(
    'Task 106 (open), started 2026-11-12 09:50 UTC: 2 turns, 10,000 input tokens (400 uncached, 9,000 read from the cache, 600 written to it), 200 output tokens, about $0.0050; 1 tool call; 90 tokens injected by Cortex.',
  );
  await tabTo(page, '.chart-input');
  await page.keyboard.press('Home');
  await expect(readout).toHaveText(
    'Task 101 (success), started 2026-11-10 09:05 UTC: 6 turns, 36,000 input tokens (2,000 uncached, 30,000 read from the cache, 4,000 written to it), 1,200 output tokens, about $0.0150; 9 tool calls; 350 tokens injected by Cortex.',
  );
  await page.keyboard.press('ArrowRight');
  await expect(readout).toHaveText(
    'Task 102 (failure, held out), started 2026-11-10 09:40 UTC: 3 turns, 13,000 input tokens (500 uncached, 12,000 read from the cache, 500 written to it), 400 output tokens, about $0.0075; 2 tool calls; 0 tokens injected by Cortex.',
  );
  await expect(page.locator('.chart-input')).toHaveAttribute(
    'aria-valuetext',
    (await readout.textContent()) ?? '',
  );
  await expect(section.getByTestId('receipts-chart').locator('.section-note')).toHaveText(
    '4 tasks, oldest on the left; 2 more have no usage Cortex reads. Point at a mark or use the arrow keys to read one.',
  );
  // The pointer explains the task nearest it: the newest at the right end, the oldest at the left.
  const box = await section.locator('.chart-plots').boundingBox();
  if (box === null) throw new Error('the plots are not drawn');
  await page.mouse.move(box.x + box.width - 1, box.y + box.height / 2);
  await expect(readout).toContainText('Task 106 (open)');
  await page.mouse.move(box.x + 1, box.y + box.height / 2);
  await expect(readout).toContainText('Task 101 (success)');
  // Each plot draws one mark a task.
  await expect(section.locator('.chart-svg').first().locator('circle')).toHaveCount(4);
  expect(record.errors).toEqual([]);
});

test('a session of more tasks than the daemon lists counts them all and says the chart is cut', async ({
  page,
}) => {
  const record = await openMap(page, { ...smallFixture(Date.now()), receipts: longReceipts() });
  const section = await openReceipts(page);
  // The daemon listed the newest 200 of 250 tasks; the session's line counts all 250.
  await expect(section.locator('.receipt-meta').first()).toHaveText(
    'Started 2026-11-10 09:00 UTC, 250 tasks.',
  );
  await expect(section.getByTestId('receipts-chart').locator('.section-note')).toHaveText(
    'The newest 60 of 250 tasks, oldest on the left. Point at a mark or use the arrow keys to read one.',
  );
  // The newest task, 250, started 250 minutes after 09:00: 13:10.
  await expect(section.getByTestId('receipts-readout')).toContainText(
    'Task 250 (success), started 2026-11-10 13:10 UTC: 1 turn, 1,100 input tokens',
  );
  await expect(section.locator('.chart-svg').first().locator('circle')).toHaveCount(60);
  expect(record.errors).toEqual([]);
});

test('a comparison sent with 29 tasks a side is refused, never shown', async ({ page }) => {
  const record = await openMap(page, {
    ...smallFixture(Date.now()),
    receipts: shortReceiptsWithComparison(),
  });
  await tabTo(page, '[data-testid="overview-receipts"] .section-toggle');
  await page.keyboard.press('Enter');
  const section = receiptsSection(page);
  await expect(section.getByTestId('receipts-error')).toContainText(
    'The receipts could not be loaded: Cortex sent an answer this page cannot read (receipts.recorded.holdout.comparison: given before each arm has enough tasks).',
  );
  await expect(section.getByTestId('receipts-headline')).toHaveCount(0);
  await expectNoClaim(section);
  expect(record.errors).toEqual([]);
});

test('30 tasks a side show the comparison with its intervals and sample sizes', async ({
  page,
}) => {
  const record = await openMap(page, smallFixture(Date.now()));
  const section = await openReceipts(page);
  await expect(section.getByTestId('receipts-headline').locator('.receipts-lead')).toHaveText(
    'Holdout comparison: 30 tasks ended with Cortex and 30 held out; each difference is with Cortex minus held out, its 95% bootstrap interval in brackets.',
  );
  // 0.8 - 0.7 is +0.10; every interval but the input tokens' holds 0.
  const measures = section.getByTestId('receipts-measure');
  await expect(measures.locator('.measure-values')).toHaveText([
    '0.80 with Cortex, 0.70 held out',
    'difference +0.10 (-0.10 to +0.30)',
    '21,000 with Cortex, 34,000 held out',
    'difference -13,000 (-20,000 to -5,000)',
    '900 with Cortex, 1,500 held out',
    'difference -600 (-1,100 to +100)',
    '4 with Cortex, 6 held out',
    'difference -2 (-3 to 0)',
    '5.5 with Cortex, 7 held out',
    'difference -1.5 (-3 to +0.5)',
  ]);
  await expect(measures.locator('.measure-note')).toHaveCount(4);
  await expect(measures.nth(1).locator('.measure-note')).toHaveCount(0);
  await expect(section.getByTestId('receipts-headline').locator('.section-note')).toHaveText(
    'Medians from 30 successful tasks with Cortex and 30 held out whose usage was read.',
  );
  await expect(section.getByTestId('receipts-prices')).toHaveText(
    'Prices: as of 2026-11-01, from your price file. Every cost is an estimate.',
  );
  expect((await page.locator('body').textContent()) ?? '').not.toMatch(/sav(e|ed|es|ing)\b/i);
  expect(record.errors).toEqual([]);
});

test('the receipts are asked for only when the section opens, and a failure says so', async ({
  page,
}) => {
  const asked: string[] = [];
  page.on('request', (request) => {
    if (request.url().endsWith('/api/receipts')) asked.push(request.url());
  });
  const record = await openMap(page, smallFixture(Date.now()));
  expect(asked).toEqual([]);
  await page.route('**/api/receipts', (route) => route.fulfill({ status: 500, body: '' }));
  await tabTo(page, '[data-testid="overview-receipts"] .section-toggle');
  await page.keyboard.press('Enter');
  const section = receiptsSection(page);
  await expect(section.getByTestId('receipts-error')).toHaveText(
    'The receipts could not be loaded: the daemon answered 500. Check that Cortex is still running (cortex status in a terminal), then try again.',
  );
  await page.unroute('**/api/receipts');
  await section.getByRole('button', { name: 'Try again' }).click();
  await expect(section.getByTestId('receipts-headline')).toBeVisible();
  expect(asked).toHaveLength(2);
  expect(record.errors.filter((error) => !error.includes('500'))).toEqual([]);
});
