// How many steps each question the map answers takes with the keyboard alone, on the ten-file
// repository whose answers are known by construction: every key pressed (each typed character
// counts as one) and every screen the person has to look at (the map with its overview is the
// first; an opened panel or popover is one more). Each row asserts the answer's text and the
// counts, so an answer that gets harder to reach turns this red; the table is printed at the end.
import { expect, test, type Page } from '@playwright/test';
import { smallFixture, threeSessions, type Fixture } from './fixtures';
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
  fixture: (nowMs: number) => Fixture = smallFixture,
): void {
  test(title, async ({ page }) => {
    const record = await openMap(page, fixture(Date.now()));
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

// The panel's header and numbers hold three stops before its sections: "Open in editor" and the
// commits and tokens numbers, which explain what they count on focus.
question('Why is book.rs the way it is?', { keys: 17, screens: 2 }, async (page, steps) => {
  await openFile(page, steps, 'book');
  await steps.tabTo(filePanel(page).getByRole('button', { name: /^History/ }));
  await steps.press('Enter');
  const subject = filePanel(page).getByTestId('panel-history').locator('.history-subject').first();
  await expect(subject).toHaveText('Keep cancelled orders in the book');
  return (await subject.textContent()) ?? '';
});

question(
  'What did Cortex tell agents about book.rs, and did it help?',
  { keys: 18, screens: 2 },
  async (page, steps) => {
    await openFile(page, steps, 'book');
    await steps.tabTo(filePanel(page).getByRole('button', { name: /^What Cortex told agents/ }));
    await steps.press('Enter');
    const first = filePanel(page).getByTestId('panel-told').getByRole('listitem').first();
    await expect(first).toContainText('then read or edited');
    return (await first.textContent()) ?? '';
  },
);

question(
  'Where are agents working right now?',
  { keys: 0, screens: 1 },
  async (page) => {
    const rows = page.getByTestId('overview-live').getByRole('listitem');
    await expect(rows).toHaveText([
      'Claude Code 3f2a… editing book.rs',
      'Codex 9b1c… reading invoice.rs',
      'Cursor c0de… claimed orders.rs',
    ]);
    return '3 sessions: Claude Code on book.rs, Codex on invoice.rs, Cursor on tests/orders.rs';
  },
  (nowMs) => ({ ...smallFixture(nowMs), live: threeSessions(nowMs) }),
);

/** Picks the view of `key` and opens the legend, which names the view and its scale. */
async function openView(page: Page, steps: Steps, key: string, title: string): Promise<void> {
  await steps.press(key);
  await steps.press('l');
  steps.sees();
  await expect(page.getByTestId('legend-view')).toHaveText(`View: ${title}`);
}

/** A legend entry's name and count, as one string. */
async function entry(page: Page, key: string): Promise<string> {
  const row = page.getByTestId(`legend-${key}`);
  const name = (await row.locator('.legend-name').textContent()) ?? '';
  const count = (await row.locator('.legend-count').textContent()) ?? '';
  return `${name}: ${count}`;
}

question('What are the hotspots? (the Heat view)', { keys: 2, screens: 2 }, async (page, steps) => {
  await openView(page, steps, '3', 'Heat');
  // Tokens 900 to 29,910 on a log scale: book.rs (18,300) and payment.rs (29,910) are hottest.
  expect(await entry(page, 'heat-4')).toBe('18,300 to 29,910 tokens: 2');
  expect(await entry(page, 'heat-none')).toBe('No tokens attributed: 3');
  return '2 files at 18,300 to 29,910 tokens; 3 with none';
});

question(
  'What have agents explored, and what have they never looked at? (the Fog view)',
  { keys: 2, screens: 2 },
  async (page, steps) => {
    await openView(page, steps, '2', 'Fog');
    expect(await entry(page, 'fog-lit')).toBe('Read by an agent: 7');
    expect(await entry(page, 'fog-dim')).toBe('Never read: 3');
    return '7 files read, 3 in fog';
  },
);

question(
  'Which files are tied to the most others? (the Coupling view)',
  { keys: 2, screens: 2 },
  async (page, steps) => {
    await openView(page, steps, '4', 'Coupling');
    // Links per file: payment.rs 5, book.rs 4, five files 2, three files 1.
    expect(await entry(page, 'coupling-4')).toBe('4 to 5 links: 2');
    expect(await entry(page, 'coupling-1')).toBe('1 link: 3');
    return 'book.rs and payment.rs, with 4 and 5 links';
  },
);

question(
  'What changed recently? (the Recent view)',
  { keys: 2, screens: 2 },
  async (page, steps) => {
    await openView(page, steps, '5', 'Recent');
    expect(await entry(page, 'recent-lit')).toBe('Read or edited in the last 24 hours: 2');
    return '2 files read or edited in the last 24 hours';
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
