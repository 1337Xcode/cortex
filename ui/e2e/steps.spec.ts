// How many steps each question the map answers takes with the keyboard alone, on the ten-file
// repository whose answers are known by construction: every key pressed (each typed character
// counts as one) and every screen the person has to look at (the map with its overview is the
// first; an opened panel or popover is one more). Each row asserts the answer's text and the
// counts, so an answer that gets harder to reach turns this red; the table is printed at the end.
import { expect, test, type Page } from '@playwright/test';
import { smallFixture } from './fixtures';
import { filePanel, openMap } from './helpers';

/** Keys pressed and screens seen while answering one question. */
class Steps {
  keys = 0;
  screens = 1;

  constructor(private readonly page: Page) {}

  /** Presses one key. */
  async press(key: string): Promise<void> {
    this.keys += 1;
    await this.page.keyboard.press(key);
  }

  /** Types `text`, one key per character. */
  async type(text: string): Promise<void> {
    this.keys += text.length;
    await this.page.keyboard.type(text);
  }

  /** Presses Tab until `target` has the focus, at most `limit` times. */
  async tabTo(target: ReturnType<Page['locator']>, limit = 30): Promise<void> {
    for (let i = 0; i < limit; i += 1) {
      if (await target.evaluate((element) => element === document.activeElement)) return;
      await this.press('Tab');
    }
    throw new Error('Tab never reached the target');
  }

  /** Counts one more screen: a panel or a popover the answer is on. */
  sees(): void {
    this.screens += 1;
  }
}

/** One row of the printed table. */
interface Row {
  readonly question: string;
  readonly answer: string;
  readonly keys: number;
  readonly screens: number;
}

const rows: Row[] = [];

/** Runs one question from a freshly opened map and records its row. */
function question(
  title: string,
  target: { readonly keys: number; readonly screens: number },
  answer: (page: Page, steps: Steps) => Promise<string>,
): void {
  test(title, async ({ page }) => {
    const record = await openMap(page, smallFixture(Date.now()));
    const steps = new Steps(page);
    const text = await answer(page, steps);
    rows.push({ question: title, answer: text, keys: steps.keys, screens: steps.screens });
    expect({ keys: steps.keys, screens: steps.screens }).toEqual(target);
    expect(record.errors).toEqual([]);
  });
}

/** Searches for `text` and opens the first result's panel. */
async function openFile(page: Page, steps: Steps, text: string): Promise<void> {
  await steps.press('/');
  await steps.type(text);
  await expect(
    page.getByRole('list', { name: 'Search results' }).getByRole('button').first(),
  ).toBeVisible();
  await steps.press('Enter');
  await expect(page.locator('#panel-title')).toBeFocused();
  steps.sees();
}

test.describe.configure({ mode: 'serial' });

question(
  'What is in this codebase and how is it organised?',
  { keys: 0, screens: 1 },
  async (page) => {
    const answer = page.getByTestId('summary-indexed');
    await expect(answer).toHaveText('10 files in 5 folders.');
    return (await answer.textContent()) ?? '';
  },
);

question(
  'Where is the code for X? (the symbol OrderBook)',
  { keys: 11, screens: 2 },
  async (page, steps) => {
    await openFile(page, steps, 'OrderBook');
    const path = filePanel(page).locator('.panel-path');
    await expect(path).toHaveText('src/orders/book.rs');
    return (await path.textContent()) ?? '';
  },
);

question(
  'If I change book.rs, what else usually changes?',
  { keys: 6, screens: 2 },
  async (page, steps) => {
    await openFile(page, steps, 'book');
    const first = filePanel(page)
      .getByTestId('panel-links-co_change')
      .getByRole('listitem')
      .first();
    await expect(first).toHaveText('mod.rs12 commits');
    return 'mod.rs, in 12 commits';
  },
);

question('What are the hotspots?', { keys: 0, screens: 1 }, async (page) => {
  const first = page.getByTestId('overview-busiest').getByRole('listitem').first();
  await expect(first).toHaveText('payment.rs29,910 tokens');
  return 'payment.rs, 29,910 tokens attributed';
});

question(
  'What have agents explored, and what have they never looked at?',
  { keys: 1, screens: 2 },
  async (page, steps) => {
    await expect(page.getByTestId('summary-explored')).toHaveText(
      'Agents have read 7 of the 10 files (70%).',
    );
    await steps.press('l');
    steps.sees();
    const unread = page.getByTestId('legend-unread').locator('.legend-count');
    await expect(unread).toHaveText('3');
    return '7 of 10 read; 3 never read';
  },
);

question('What changed recently?', { keys: 1, screens: 2 }, async (page, steps) => {
  await steps.press('l');
  steps.sees();
  const recent = page.getByTestId('legend-recent').locator('.legend-count');
  await expect(recent).toHaveText('2');
  return '2 files read or edited in the last 24 hours';
});

question(
  'Which facts apply to book.rs, and which went stale?',
  { keys: 6, screens: 2 },
  async (page, steps) => {
    await openFile(page, steps, 'book');
    const facts = filePanel(page).getByTestId('panel-facts').getByRole('listitem');
    await expect(facts).toHaveText([
      'ActiveinvariantOrder totals are kept in integer cents.',
      'SuspectconventionA cancelled order keeps its book entry.',
    ]);
    return '2 facts, 1 suspect';
  },
);

question('Why is book.rs the way it is?', { keys: 14, screens: 2 }, async (page, steps) => {
  await openFile(page, steps, 'book');
  await steps.tabTo(filePanel(page).getByRole('button', { name: /^History/ }));
  await steps.press('Enter');
  const subject = filePanel(page).getByTestId('panel-history').locator('.history-subject').first();
  await expect(subject).toHaveText('Keep cancelled orders in the book');
  return (await subject.textContent()) ?? '';
});

question(
  'What did Cortex tell agents about book.rs, and did it help?',
  { keys: 15, screens: 2 },
  async (page, steps) => {
    await openFile(page, steps, 'book');
    await steps.tabTo(filePanel(page).getByRole('button', { name: /^What Cortex told agents/ }));
    await steps.press('Enter');
    const first = filePanel(page).getByTestId('panel-told').getByRole('listitem').first();
    await expect(first).toContainText('then read or edited');
    return (await first.textContent()) ?? '';
  },
);

test.afterAll(() => {
  const lines = [
    '| Question | Answer shown | Keys | Screens |',
    '| --- | --- | --- | --- |',
    ...rows.map((row) => `| ${row.question} | ${row.answer} | ${row.keys} | ${row.screens} |`),
  ];
  process.stdout.write(`${lines.join('\n')}\n`);
});
