// The map's acceptance against a real Cortex daemon: each question the map is for, answered with
// the keyboard alone on the scripted repository (repository.ts) after the scripted story
// (story.ts), and each answer asserted as the literal text the construction makes true. The
// counts behind each answer are worked out by hand in the comment above its check; nothing
// expected here was read off the map first.
//
// The story in short: six files in three folders (calc/money.py 12 lines, calc/ops.py 13,
// calc/rates.py 5, docs/ops.md 3, main.py 3, tests/test_ops.py 14). One Claude Code session reads
// calc/ops.py, tests/test_ops.py and calc/money.py, edits calc/ops.py (and is warned of
// docs/ops.md), runs the tests, reads docs/ops.md and ends its answer; it is still live. The user
// notes a rule about `divide` (made suspect by a later change) and one about `Ledger`.
import { expect, test, type Page } from '@playwright/test';
import { filePanel, tabTo } from '../helpers';
import { expectClean, openFile, openRealMap, searchAgain } from './open';
import { RATES, textOf } from './repository';
import { hook, readOf } from './session';
import { COMPANION_WARNING } from './story';
import { storyDays } from './state';

/** A legend entry's name and count, as `name: count`. */
async function legendEntry(page: Page, key: string): Promise<string> {
  const row = page.getByTestId(`legend-${key}`);
  const name = (await row.locator('.legend-name').textContent()) ?? '';
  const count = (await row.locator('.legend-count').textContent()) ?? '';
  return `${name}: ${count}`;
}

/** Picks the view of `key`, opens the legend and waits until it names the view `title`. */
async function openView(page: Page, key: string, title: string): Promise<void> {
  await page.keyboard.press(key);
  await page.keyboard.press('l');
  await expect(page.getByTestId('legend-view')).toHaveText(`View: ${title}`);
}

/**
 * The tokens Cortex estimates a text it injected takes, by its documented rule: the text's UTF-8
 * bytes over four, rounded up.
 */
function estimatedTokens(text: string): number {
  return Math.ceil(Buffer.byteLength(text, 'utf8') / 4);
}

// Folders are counted at any depth, the root not counted: calc, docs and tests. In the default
// view each file shows its most pressing mark: calc/ops.py has a suspect fact (1); the other three
// files the session read were read in the last 24 hours (3); main.py and calc/rates.py were never
// read (2); no file is left plain (0). 1 + 3 + 2 = 6, every file.
test('What is in this codebase and how is it organised?', async ({ page }) => {
  const opened = await openRealMap(page);
  await expect(page.getByTestId('summary-indexed')).toHaveText('6 files in 3 folders.');
  await page.keyboard.press('l');
  await expect(page.getByTestId('legend-view')).toHaveText('View: Default');
  await expect(page.getByTestId('legend-folders').locator('.legend-count')).toHaveText('3');
  expect(await legendEntry(page, 'suspect')).toBe('Has a suspect fact: 1');
  expect(await legendEntry(page, 'recent')).toBe('Read or edited in the last 24 hours: 3');
  expect(await legendEntry(page, 'unread')).toBe('Never read by an agent: 2');
  expect(await legendEntry(page, 'plain')).toBe('Other files: 0');
  expectClean(opened);
});

// `Ledger` is the class calc/money.py defines; the file has 12 lines (6, then 3 more, then 3).
test('Where is the code for X? (the symbol Ledger)', async ({ page }) => {
  const opened = await openRealMap(page);
  await openFile(page, 'Ledger');
  await expect(filePanel(page).locator('.panel-path')).toHaveText('calc/money.py');
  await expect(filePanel(page)).toContainText('Lines12');
  expectClean(opened);
});

// calc/ops.py and docs/ops.md changed together in commits 2, 5 and 8, and no other two files ever
// changed in one commit: the one co-change link, 3 commits. The links the map keeps per file
// (Coupling): calc/ops.py has that link and a used-together link to each of the three files the
// task read (the task's success added 0.1 x 1.0 for the edited file x 0.2 for a file only read =
// 0.02 to each such pair; a pair of two files it only read got 0.1 x 0.2 x 0.2 = 0.004, below
// the 0.01 a link keeps), 4; docs/ops.md 2 (the co-change and the used-together link);
// calc/money.py and tests/test_ops.py 1 each; main.py and calc/rates.py none. The steps, ceil(4 x
// links / 4): 4 links step 4, 2 links step 2, 1 link step 1.
test('If I change ops.py, what else usually changes?', async ({ page }) => {
  const opened = await openRealMap(page);
  await expect(page.getByTestId('overview-pairs').getByRole('listitem')).toHaveText(
    ['ops.py and ops.md 3 commits'],
    { useInnerText: true },
  );
  await expect(page.getByTestId('summary-pair')).toHaveText(
    'Strongest coupling: ops.py and ops.md changed together in 3 commits.',
  );
  await openView(page, '4', 'Coupling');
  expect(await legendEntry(page, 'coupling-4')).toBe('4 links: 1');
  expect(await legendEntry(page, 'coupling-2')).toBe('2 links: 1');
  expect(await legendEntry(page, 'coupling-1')).toBe('1 link: 2');
  expect(await legendEntry(page, 'coupling-none')).toBe('No learned links: 2');
  await openFile(page, 'ops.py');
  await expect(
    filePanel(page).getByTestId('panel-links-co_change').getByRole('listitem'),
  ).toHaveText(['ops.md 3 commits'], { useInnerText: true });
  expectClean(opened);
});

// Each response counts in full for the file the session last read or edited before it (story.ts):
// calc/ops.py 3,200 + 4,300 = 7,500, calc/money.py 2,100, tests/test_ops.py 1,100, docs/ops.md
// 550; the folder calc 7,500 + 2,100 = 9,600, more than tests (1,100) and docs (550). Heat steps
// a file's tokens v on a log scale from 550 to 7,500: ceil(4 x ln(v / 550) / ln(7,500 / 550)),
// at least 1, where ln(7,500 / 550) = 2.613: 550 gives step 1; 1,100, ceil(4 x 0.693 / 2.613) =
// ceil(1.06) = 2; 2,100, ceil(4 x 1.340 / 2.613) = ceil(2.05) = 3; 7,500, step 4.
test('What are the hotspots?', async ({ page }) => {
  const opened = await openRealMap(page);
  await expect(page.getByTestId('overview-busiest').getByRole('listitem')).toHaveText(
    [
      'ops.py 7,500 tokens',
      'money.py 2,100 tokens',
      'test_ops.py 1,100 tokens',
      'ops.md 550 tokens',
    ],
    { useInnerText: true },
  );
  await expect(page.getByTestId('summary-busiest')).toHaveText(
    'Most worked on in the last 7 days: calc/, whose own files took 9,600 tokens attributed.',
  );
  await openView(page, '3', 'Heat');
  expect(await legendEntry(page, 'heat-4')).toBe('7,500 tokens: 1');
  expect(await legendEntry(page, 'heat-3')).toBe('2,100 tokens: 1');
  expect(await legendEntry(page, 'heat-2')).toBe('1,100 tokens: 1');
  expect(await legendEntry(page, 'heat-1')).toBe('550 tokens: 1');
  expect(await legendEntry(page, 'heat-none')).toBe('No tokens attributed: 2');
  expectClean(opened);
});

// The session read 4 of the 6 files (calc/ops.py, tests/test_ops.py, calc/money.py, docs/ops.md):
// 4 / 6 = 66.7%, shown to the nearest whole, 67%. main.py and calc/rates.py are in fog.
test('What have agents explored, and what have they never looked at?', async ({ page }) => {
  const opened = await openRealMap(page);
  await expect(page.getByTestId('summary-explored')).toHaveText(
    'Agents have read 4 of the 6 files (67%).',
  );
  await openView(page, '2', 'Fog');
  expect(await legendEntry(page, 'fog-lit')).toBe('Read by an agent: 4');
  expect(await legendEntry(page, 'fog-dim')).toBe('Never read: 2');
  expectClean(opened);
});

// Fact 1, `divide rounds its result to cents.` (a convention, the default kind), is about
// calc/ops.py's `divide`, which changed after it was noted: suspect. Fact 2, `Ledger amounts are
// whole cents.`, an invariant about calc/money.py's `Ledger`, is active. Two files have facts; one
// has a suspect fact.
test('Which rules and facts apply here, and which went stale?', async ({ page }) => {
  const opened = await openRealMap(page);
  await expect(page.getByTestId('overview-attention').getByRole('listitem')).toHaveText(
    ['ops.py 1 suspect fact'],
    { useInnerText: true },
  );
  await openView(page, '2', 'Fog');
  expect(await legendEntry(page, 'badge')).toBe('Has facts: 2');
  expect(await legendEntry(page, 'mark-suspect')).toBe('Second ring: 1');
  await openFile(page, 'ops.py');
  await expect(filePanel(page).getByTestId('panel-facts').getByRole('listitem')).toHaveText([
    'Suspectconventiondivide rounds its result to cents.Confirm againRetire',
  ]);
  await searchAgain(page, 'Ledger');
  await expect(filePanel(page).getByTestId('panel-facts').getByRole('listitem')).toHaveText([
    'ActiveinvariantLedger amounts are whole cents.Retire',
  ]);
  expectClean(opened);
});

// calc/money.py's 12 lines come from three commits, each adding lines the next kept: lines 1 to 6
// from "Add the ledger" (09:20), 7 to 9 from "Total the ledger" (09:50), 10 to 12 from "Keep ledger
// amounts in cents" (10:20), all on 2026-11-01; newest first. The history worker is on
// (`history.why`), and upkeep asked for the blame of each file the session worked on with 3
// commits or more.
test('Why is this code the way it is?', async ({ page }) => {
  const opened = await openRealMap(page);
  await openFile(page, 'Ledger');
  await tabTo(page, '[data-testid="panel-history"] .section-toggle');
  await page.keyboard.press('Enter');
  const history = filePanel(page).getByTestId('panel-history');
  await expect(history.locator('.history-subject')).toHaveText([
    'Keep ledger amounts in cents',
    'Total the ledger',
    'Add the ledger',
  ]);
  await expect(history.locator('.history-meta')).toHaveText([
    '2026-11-01',
    '2026-11-01',
    '2026-11-01',
  ]);
  expectClean(opened);
});

// The edit of calc/ops.py was warned that docs/ops.md changed with it in 3 of its 4 commits; the
// session then read docs/ops.md, so its end marked the warning followed. The strength is the
// share of calc/ops.py's commits that changed both, recent commits weighing more (a commit k
// commits back weighs 2^(-k/500)): commits 0, 2, 5 and 8 back, 3 of them shared, (0.99723 +
// 0.99309 + 0.98897) / (1 + 0.99723 + 0.99309 + 0.98897) = 0.7487, shown as 0.75.
test('What did Cortex tell the agent about this file, and did it help?', async ({ page }) => {
  const opened = await openRealMap(page);
  const day = storyDays(opened.state);
  await openFile(page, 'ops.md');
  await tabTo(page, '[data-testid="panel-told"] .section-toggle');
  await page.keyboard.press('Enter');
  await expect(filePanel(page).getByTestId('panel-told').getByRole('listitem')).toHaveText([
    new RegExp(`^Warning(${day}), then read or edited$`),
  ]);
  await page.keyboard.press('Escape');
  await tabTo(page, '[data-testid="overview-warnings"] .section-toggle');
  await page.keyboard.press('Enter');
  const warnings = page.getByTestId('overview-warnings');
  await expect(warnings.locator('.stack-what')).toHaveText(['ops.py usually changes with ops.md']);
  await expect(warnings.locator('.stack-meta')).toHaveText([
    new RegExp(`^Followed · strength 0\\.75 · (${day})$`),
  ]);
  expectClean(opened);
});

// No task was held out (`holdout.share = 0`): 0 held out and 1 treated, short of 30 a side. The
// task's 5 responses took 3,000 + 1,000 + 2,000 + 4,000 + 500 = 10,500 input tokens, none cached,
// and 200 + 100 + 100 + 300 + 50 = 750 output tokens; it made 6 tool calls (four reads, the edit
// and the test run). Cortex injected the primer and, in the task, the warning: each estimated as
// its UTF-8 bytes over four, rounded up. No work order was given, so none is labelled; the one
// warning named one file, which was then read: 1 of 1. The cost is left out: it depends on the
// price table this Cortex ships.
test('Is Cortex worth it?', async ({ page }) => {
  const opened = await openRealMap(page);
  const day = storyDays(opened.state);
  const primer = estimatedTokens(opened.state.primer);
  const warning = estimatedTokens(COMPANION_WARNING);
  await tabTo(page, '[data-testid="overview-receipts"] .section-toggle');
  await page.keyboard.press('Enter');
  const receipts = page.getByTestId('overview-receipts');
  await expect(receipts.getByTestId('receipts-headline').locator('.receipts-lead')).toHaveText(
    'Not enough data yet: 0 of 30 held-out and 1 of 30 treated tasks so far.',
  );
  await expect(receipts.getByTestId('receipts-precision')).toHaveText(
    'Injection precision: no work order given is labelled yet.',
  );
  await expect(receipts.getByTestId('receipts-warnings')).toHaveText(
    'Warning precision: 1.00, 1 of 1 file warnings named and marked were then read or edited.',
  );
  await tabTo(page, '.chart-input');
  await page.keyboard.press('Home');
  await expect(receipts.getByTestId('receipts-readout')).toHaveText(
    new RegExp(
      `^Task 1 \\(success\\), started (${day}) \\d\\d:\\d\\d UTC: 5 turns, 10,500 input tokens ` +
        `\\(10,500 uncached, 0 read from the cache, 0 written to it\\), 750 output tokens, ` +
        `about \\$\\d+\\.\\d+; 6 tool calls; ${warning} tokens injected by Cortex\\.$`,
    ),
  );
  await expect(receipts.getByTestId('receipts-overhead')).toHaveText(
    `${primer + warning} tokens injected by Cortex (primer: ${primer} tokens), against 10,500 input tokens in total.`,
  );
  expectClean(opened);
});

// The Recent view lights the 4 files the session read or edited, all in the last 24 hours; the
// session has not ended, so it is live, its last step a read of docs/ops.md. Then the session
// reads calc/rates.py, and the live frame moves its marker there while the page stays open. This
// check comes last: the read it adds would change what the checks above count.
test('What changed recently, and where are agents working right now?', async ({ page }) => {
  const opened = await openRealMap(page);
  const live = page.getByTestId('overview-live').getByRole('listitem');
  await expect(live).toHaveText(['Claude Code 0000… reading ops.md'], { useInnerText: true });
  await openView(page, '5', 'Recent');
  expect(await legendEntry(page, 'recent-lit')).toBe('Read or edited in the last 24 hours: 4');
  expect(await legendEntry(page, 'recent-dim')).toBe('Not in the last 24 hours: 2');
  expect(await legendEntry(page, 'marker')).toBe('An agent working now: 1');
  const read = readOf(opened.state, RATES.path, textOf(RATES), RATES.lines.length);
  expect(await hook(opened.state, read)).toBeNull();
  await expect(live).toHaveText(['Claude Code 0000… reading rates.py'], {
    useInnerText: true,
    timeout: 30_000,
  });
  expectClean(opened);
});
