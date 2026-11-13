// The overview's last panels and the file panel's new parts, in a browser on the ten-file
// repository: the activity (sentences, times, flying to a file, opening while a session runs, a
// failure), the warnings (open first, in words, flying to the named file), the daemon's health,
// facts confirmed and retired through the POST the daemon takes (its Origin as Chromium sends it,
// the busy state, the retire step, a refusal in the daemon's words, the panel read again), the
// file's symbols with the lines work orders selected, the history worker off, a generated file's
// commits, and the quiet refresh a live frame's new position asks for.
import { expect, test, type Page } from '@playwright/test';
import {
  fileDetail,
  givenWarning,
  happened,
  liveSession,
  AGENTS,
  CLAUDE,
  smallFixture,
  threeSessions,
  workingHealth,
  type Fixture,
  type FixtureFile,
} from './fixtures';
import { filePanel, openMap, searchAndPick, type PageRecord } from './helpers';
import { factsChannel, liveChannel, MAP_ORIGIN } from './serve';

let record: PageRecord;

test.afterEach(() => {
  expect(record.errors).toEqual([]);
  expect(record.foreign).toEqual([]);
});

/** Opens a closed overview section by its title. */
async function openSection(page: Page, title: string): Promise<void> {
  const toggle = page.getByRole('button', { name: new RegExp(`^${title}`) });
  await toggle.click();
  await expect(toggle).toHaveAttribute('aria-expanded', 'true');
}

test.describe('activity', () => {
  test('opens while a session runs and says what agents did, newest first', async ({ page }) => {
    const nowMs = Date.now();
    record = await openMap(page, { ...smallFixture(nowMs), live: threeSessions(nowMs) });
    const section = page.getByTestId('overview-activity');
    const rows = section.getByRole('listitem');
    await expect(rows).toHaveCount(5);
    await expect(rows.first()).toHaveText('Claude Code 3f2a… edited book.rs 2 min ago', {
      useInnerText: true,
    });
    await expect(rows.nth(2)).toContainText('Claude Code 3f2a… ran a shell command');
    await expect(rows.nth(3)).toContainText('Codex 9b1c… read invoice.rs');
    // How to read the times is the section's last line, whole on keyboard focus.
    await section.locator('.section-footnote .clip').focus();
    await expect(page.getByRole('tooltip')).toContainText("by this computer's clock");
    await section.getByRole('button', { name: 'Show 2 more' }).click();
    await expect(rows).toHaveCount(7);
    await expect(rows.last()).toContainText('Claude Code 3f2a… was given a prompt');
    // The row's whole sentence, with the file's path and the exact time in UTC, is its name; it
    // flies to book.rs.
    const first = section.getByRole('button', {
      name: /edited src\/orders\/book\.rs, 2 min ago, at .* UTC\.$/,
    });
    await first.click();
    await expect(filePanel(page).locator('#panel-title')).toHaveText('book.rs');
  });

  test('stays closed with no session running, and loads when opened', async ({ page }) => {
    const asked: string[] = [];
    page.on('request', (request) => {
      if (request.url().endsWith('/api/activity')) asked.push(request.url());
    });
    record = await openMap(page, smallFixture(Date.now()));
    const toggle = page.getByRole('button', { name: /^Activity/ });
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(asked).toEqual([]);
    await toggle.click();
    await expect(page.getByTestId('overview-activity').getByRole('listitem')).toHaveCount(5);
    expect(asked).toHaveLength(1);
    await expect(toggle).toContainText('7');
  });

  test("keeps the person's choice once they open or close it, as sessions start and stop", async ({
    page,
  }) => {
    const nowMs = Date.now();
    record = await openMap(page, { ...smallFixture(nowMs), live: threeSessions(nowMs) });
    const toggle = page.getByRole('button', { name: /^Activity/ });
    await expect(toggle).toHaveAttribute('aria-expanded', 'true');
    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
    const live = liveChannel(page);
    live.push({ version: 1, sessions: [] });
    const sessions = page.getByRole('button', { name: /^Live sessions/ });
    await expect(sessions).toHaveText('Live sessions0');
    live.push({
      version: 1,
      sessions: [liveSession('3f2a9c1e', AGENTS.claude, [[4, 'read']], nowMs + 1000)],
    });
    await expect(sessions).toHaveText('Live sessions1');
    await expect(toggle).toHaveAttribute('aria-expanded', 'false');
  });

  test('says what failed and tries again', async ({ page }) => {
    record = await openMap(page, smallFixture(Date.now()));
    await page.route('**/api/activity', (route) => route.fulfill({ status: 500, body: '' }));
    await openSection(page, 'Activity');
    await expect(page.getByTestId('activity-error')).toHaveText(
      'The activity could not be loaded: the daemon answered 500. Check that Cortex is still running (cortex status in a terminal), then try again.',
    );
    await page.unroute('**/api/activity');
    await page.getByTestId('overview-activity').getByRole('button', { name: 'Try again' }).click();
    await expect(page.getByTestId('overview-activity').getByRole('listitem')).toHaveCount(5);
    // The failed load logged its 500; the check after each test reads only the page's own errors.
    record.errors.length = 0;
  });
});

test.describe('a file the map does not show yet', () => {
  test('is named by its path, never linked to the file at its id, and the map reads its data again', async ({
    page,
  }) => {
    const nowMs = Date.now();
    const graphs: string[] = [];
    page.on('request', (request) => {
      if (request.url().endsWith('/api/graph')) graphs.push(request.url());
    });
    // The daemon added src/orders/aaa.rs, so its ids moved on: its 4 is the new file (book.rs on
    // screen), its 5 is book.rs (mod.rs on screen). vendor/gen.rs is a file the map does not list.
    const activity = {
      version: 1,
      events: [
        happened(nowMs, 60_000, 'edit', CLAUDE, [4, 'src/orders/aaa.rs']),
        happened(nowMs, 120_000, 'read', CLAUDE, [null, 'vendor/gen.rs']),
        happened(nowMs, 180_000, 'read', CLAUDE, [5, 'src/orders/book.rs']),
        happened(nowMs, 240_000, 'prompt', CLAUDE, [null, null]),
      ],
    };
    const warnings = {
      version: 1,
      warnings: [
        givenWarning(
          nowMs,
          'cochange',
          [
            [5, 'src/orders/book.rs'],
            [4, 'src/orders/aaa.rs'],
          ],
          0.004,
          'open',
        ),
      ],
    };
    record = await openMap(page, {
      ...smallFixture(nowMs),
      live: threeSessions(nowMs),
      activity,
      warnings,
    });
    const section = page.getByTestId('overview-activity');
    await expect(section.locator('.stack-what')).toHaveText([
      'Claude Code 3f2a… edited src/orders/aaa.rs',
      'Claude Code 3f2a… read vendor/gen.rs',
      'Claude Code 3f2a… read book.rs',
      'Claude Code 3f2a… was given a prompt',
    ]);
    await expect(section.locator('.stack-meta').nth(0)).toHaveText(
      '1 min ago · not on the map yet',
    );
    await expect(section.locator('.stack-meta').nth(1)).toHaveText('2 min ago · not on the map');
    // Neither is a button: the file at id 4 on screen is book.rs, which the event was not about.
    await expect(section.locator('.stack-row').nth(0)).toHaveJSProperty('tagName', 'P');
    await expect(section.locator('.stack-row').nth(1)).toHaveJSProperty('tagName', 'P');
    // The answer found the snapshot older than the daemon's ids, so the map reads it again.
    await expect.poll(() => graphs.length, { timeout: 10_000 }).toBeGreaterThanOrEqual(2);
    await openSection(page, 'Warnings');
    const warning = page.getByTestId('overview-warnings').locator('.stack-row').first();
    await expect(warning.locator('.stack-what')).toHaveText(
      'book.rs usually changes with src/orders/aaa.rs',
    );
    await expect(warning.locator('.stack-meta')).toContainText('Open · strength under 0.01 · ');
    await expect(warning.locator('.stack-meta')).toContainText('not on the map yet');
    await expect(warning).toHaveJSProperty('tagName', 'P');
    // The book.rs event, its id out of step, is found again by its path and flies to book.rs.
    await section
      .getByRole('button', { name: /^Claude Code 3f2a… read src\/orders\/book\.rs,/ })
      .click();
    await expect(filePanel(page).locator('#panel-title')).toHaveText('book.rs');
  });

  test('a search result is found by its path, and one the map does not hold yet is no button', async ({
    page,
  }) => {
    const graphs: string[] = [];
    page.on('request', (request) => {
      if (request.url().endsWith('/api/graph')) graphs.push(request.url());
    });
    record = await openMap(page, smallFixture(Date.now()));
    // The daemon added src/orders/aaa.rs, so its ids moved on: its 4 is the new file (book.rs on
    // screen), its 5 is book.rs (mod.rs on screen).
    await page.route(/\/api\/search\?/, (route) => {
      const query = new URL(route.request().url()).searchParams.get('q') ?? '';
      return route.fulfill({
        json: {
          version: 1,
          query,
          results: [
            { kind: 'file', file: 4, path: 'src/orders/aaa.rs', name: 'aaa.rs', line: null },
            { kind: 'symbol', file: 5, path: 'src/orders/book.rs', name: 'OrderBook', line: 12 },
          ],
        },
      });
    });
    await page.keyboard.press('/');
    await page.keyboard.type('o');
    const results = page.getByRole('list', { name: 'Search results' });
    const pending = results.locator('.search-result').first();
    await expect(pending).toHaveJSProperty('tagName', 'P');
    await expect(pending).toContainText('file, not on the map yet');
    await expect(pending).toContainText('src/orders/aaa.rs');
    // The symbol's file is book.rs, found by its path at 4, never mod.rs at the daemon's 5.
    await expect(results.getByRole('button')).toHaveCount(1);
    await expect(results.getByRole('button')).toHaveAccessibleName(
      'OrderBook, symbol, src/orders/book.rs:12',
    );
    // Enter picks the first result the map holds.
    await page.keyboard.press('Enter');
    await expect(filePanel(page).locator('#panel-title')).toHaveText('book.rs');
    // The answer found the snapshot older than the daemon's ids, so the map reads it again.
    await expect.poll(() => graphs.length, { timeout: 10_000 }).toBeGreaterThanOrEqual(2);
  });

  test('a summary the map refuses asks for the map and the summary again', async ({ page }) => {
    const nowMs = Date.now();
    const graphs: string[] = [];
    page.on('request', (request) => {
      if (request.url().endsWith('/api/graph')) graphs.push(request.url());
    });
    // The summary is newer than the graph: it names a pair the map does not link.
    const fixture = smallFixture(nowMs);
    const summary = fixture.summary;
    if (summary === null || typeof summary !== 'object' || Array.isArray(summary)) {
      throw new Error('the summary is an object');
    }
    const ahead = { ...summary, strongest_pair: { from: 0, to: 9, commits: 40 } };
    record = await openMap(page, { ...fixture, summary: ahead });
    await expect(page.locator('.summary-state')).toContainText('The summary could not be loaded');
    await expect.poll(() => graphs.length, { timeout: 10_000 }).toBeGreaterThanOrEqual(2);
  });
});

test.describe('the commits behind a file', () => {
  test('cuts long references to the row and says them whole on hover and focus', async ({
    page,
  }) => {
    const nowMs = Date.now();
    const fixture = smallFixture(nowMs);
    // The longest references the daemon writes: a pull request of 20 digits and 3 issue
    // references of 100 characters, 324 characters in all.
    const refs = `#${'9'.repeat(20)}${` ${'r'.repeat(100)}`.repeat(3)}`;
    const detail = fileDetail(9, 'ui/view.ts', {
      history: [
        { subject: 'Show the long view', refs, date_s: 1762300000 },
        { subject: 'Name the view', refs: '#7', date_s: 1761000000 },
      ],
    });
    record = await openMap(page, { ...fixture, details: new Map([[9, detail]]) });
    await searchAndPick(page, 'view.ts');
    const panel = filePanel(page);
    await panel.getByRole('button', { name: /^Commits behind its lines/ }).click();
    // The long references are cut at a word before the row's end, keeping the pull request; the
    // short ones are whole.
    const shown = panel.locator('.history-refs').first();
    await expect(shown).toHaveText(/^#9{20}( r+)?…$/);
    await expect(panel.locator('.history-refs').nth(1)).toHaveText('#7');
    await shown.focus();
    await expect(page.getByRole('tooltip')).toHaveText(refs);
    await expect(panel).not.toContainText('Could not load this');
  });
});

test.describe('warnings', () => {
  test('lists the open first, each in words with its strength and what became of it', async ({
    page,
  }) => {
    record = await openMap(page, smallFixture(Date.now()));
    await openSection(page, 'Warnings');
    const section = page.getByTestId('overview-warnings');
    // How many are open and resolved is the section's last line, whole on keyboard focus.
    await section.locator('.section-footnote .clip').focus();
    await expect(page.getByRole('tooltip')).toHaveText(
      '2 warnings open and 3 resolved, among the 100 newest Cortex gave; open first.',
    );
    await expect(section.locator('.stack-what')).toHaveText([
      'book.rs usually changes with mod.rs',
      'orders.rs is a test of book.rs',
      'payment.rs usually changes with book.rs',
      'tax.rs calls a symbol of invoice.rs whose signature changed',
      'a file the map no longer lists is a test of lib.rs',
    ]);
    await expect(section.locator('.stack-meta').first()).toHaveText(/^Open · strength 0\.62 · /);
    await expect(section.locator('.stack-meta').nth(2)).toHaveText(/^Followed · strength 0\.80 · /);
    // The warning whose named file is gone is no button; the others fly to the named file.
    await expect(section.locator('p.stack-row')).toHaveCount(1);
    await section
      .getByRole('button', {
        name: /^src\/orders\/book\.rs usually changes with src\/orders\/mod\.rs\./,
      })
      .click();
    await expect(filePanel(page).locator('#panel-title')).toHaveText('mod.rs');
  });
});

test.describe('health', () => {
  test("shows the daemon's report, its hook times as its own handling", async ({ page }) => {
    const nowMs = Date.now();
    record = await openMap(page, { ...smallFixture(nowMs), health: workingHealth(nowMs) });
    await openSection(page, 'Health');
    const section = page.getByTestId('overview-health');
    await expect(section.getByRole('button', { name: /^Health/ })).toContainText('2 errors');
    await expect(section.getByTestId('health-version')).toHaveText(
      'Cortex 0.0.0, finished its first reading of the repository.',
    );
    await expect(section).toContainText(
      'Running for 2 hours; since it started, 0 hook events dropped and 1 batch of events not recorded.',
    );
    const rows = section.getByTestId('health-hooks').locator('tbody tr');
    await expect(rows).toHaveText([
      'Before a tool412850 µs2.4 ms9.1 ms',
      'Prompt3714.2 ms31.0 ms48.5 ms',
    ]);
    await expect(section.locator('caption')).toContainText('not the hook process');
    await expect(section.locator('.health-errors li').first()).toContainText(
      'the watcher lost an event (3 times, last at',
    );
    await expect(section).toContainText(
      'The graph holds 1,200 nodes and 5,400 links, as last built',
    );
    await expect(section).toContainText('The history worker is off');
    await expect(section).not.toContainText('budget');
  });
});

test.describe('health, unreadable', () => {
  test("says the daemon's report could not be read, and the rest of the map works", async ({
    page,
  }) => {
    const nowMs = Date.now();
    const health = workingHealth(nowMs);
    if (health === null || typeof health !== 'object' || Array.isArray(health)) {
      throw new Error('the working health is an object');
    }
    const daemon = health['daemon'];
    if (daemon === null || typeof daemon !== 'object' || Array.isArray(daemon)) {
      throw new Error('the working health has a report');
    }
    // One character past the length the daemon cuts the miner's detail to.
    const unreadable = { ...health, daemon: { ...daemon, git_detail: 'd'.repeat(1001) } };
    record = await openMap(page, { ...smallFixture(nowMs), health: unreadable });
    await openSection(page, 'Health');
    const section = page.getByTestId('overview-health');
    await expect(section.getByTestId('health-version')).toHaveText(
      'Cortex 0.0.0, finished its first reading of the repository.',
    );
    await expect(section.getByTestId('health-unreadable')).toHaveText(
      "The daemon's report could not be read (bad_range): health.daemon.git_detail: longer than 1000 characters. Run cortex status in a terminal to see it.",
    );
    await expect(section.getByRole('button', { name: /^Health/ })).toContainText('unknown');
  });
});

test.describe('facts', () => {
  test('confirms a candidate with a POST in the default mode, then reads the panel again', async ({
    page,
  }) => {
    record = await openMap(page, smallFixture(Date.now()));
    await searchAndPick(page, 'payment');
    const facts = filePanel(page).getByTestId('panel-facts');
    await expect(facts.getByTestId('fact-11')).toContainText('Candidate');
    // The answer waits half a second, so the busy state shows.
    await page.route('**/api/facts/**', async (route) => {
      await new Promise((resolve) => setTimeout(resolve, 500));
      await route.fallback();
    });
    await facts.getByRole('button', { name: 'Confirm fact 11' }).click();
    await expect(facts.getByRole('button', { name: 'Confirm fact 11' })).toHaveText('Confirming…');
    await expect(facts.getByRole('button', { name: 'Confirm fact 11' })).toBeDisabled();
    await expect(facts.getByTestId('fact-said')).toHaveText(
      "Fact 11 confirmed: it is active now, and Cortex may give it to agents when its evidence passes Cortex's checks.",
    );
    await expect(facts.getByTestId('fact-said')).toBeFocused();
    // The panel was read again: the fact is active now, and offers Retire.
    await expect(facts.getByTestId('fact-11')).toContainText('Active');
    await expect(facts.getByRole('button', { name: 'Retire fact 11' })).toBeVisible();
    // Chromium sent the map's own Origin and no body, which the daemon's guard asks for.
    expect(factsChannel(page).posts()).toEqual([
      { path: 'api/facts/11/confirm', origin: MAP_ORIGIN, hasBody: false },
    ]);
  });

  test('asks before retiring, keeps the fact on Escape, then retires it', async ({ page }) => {
    record = await openMap(page, smallFixture(Date.now()));
    await searchAndPick(page, 'book');
    const facts = filePanel(page).getByTestId('panel-facts');
    const retire = facts.getByRole('button', { name: 'Retire fact 3' });
    await retire.click();
    await expect(facts.getByRole('group', { name: 'Retire fact 3' })).toContainText(
      'Retire fact 3? Cortex will stop showing it here and stop giving it to agents.',
    );
    await expect(facts.getByRole('button', { name: 'Keep it' })).toBeFocused();
    await page.keyboard.press('Escape');
    // Escape kept the fact and the panel open, and gave the focus back to Retire.
    await expect(filePanel(page)).toBeVisible();
    await expect(retire).toBeFocused();
    expect(factsChannel(page).posts()).toEqual([]);
    await retire.click();
    await facts.getByRole('button', { name: 'Retire it' }).click();
    await expect(facts.getByTestId('fact-said')).toHaveText(
      'Fact 3 retired: Cortex no longer shows it or gives it to agents.',
    );
    await expect(facts.getByTestId('fact-3')).toHaveCount(0);
    await expect(facts.getByRole('button', { name: /^Facts/ })).toContainText('1');
  });

  test("says a refusal in the daemon's words, and a suspect fact can be confirmed again", async ({
    page,
  }) => {
    record = await openMap(page, smallFixture(Date.now()));
    await searchAndPick(page, 'book');
    const facts = filePanel(page).getByTestId('panel-facts');
    await expect(facts.getByTestId('fact-8').getByRole('button')).toHaveText([
      'Confirm again',
      'Retire',
    ]);
    factsChannel(page).answerNext(409, { version: 1, error: 'fact 8 was corrected as fact 12' });
    await facts.getByRole('button', { name: 'Confirm again fact 8' }).click();
    await expect(facts.getByTestId('fact-said')).toHaveText(
      'Cortex refused to confirm fact 8: fact 8 was corrected as fact 12.',
    );
    // The 409 was logged by the browser as a failed request; it is the answer under test.
    record.errors.length = 0;
  });
});

test.describe('the file panel', () => {
  test("lists a file's symbols, highlighting what work orders selected, each opening at its line", async ({
    page,
  }) => {
    record = await openMap(page, smallFixture(Date.now()));
    await searchAndPick(page, 'book');
    await filePanel(page)
      .getByRole('button', { name: /^Symbols/ })
      .click();
    const section = filePanel(page).getByTestId('panel-symbols');
    await expect(section).toContainText(
      '4 symbols, in line order, as Cortex last indexed the file. Highlighted: 2 symbols holding lines the work orders that listed this file (the latest 20 at most) selected.',
    );
    await expect(section.locator('.span-row')).toHaveText([
      'Lines 50 to 72, selected by 2 work orders.',
    ]);
    const rows = section.locator('a.symbol-row');
    await expect(rows).toHaveCount(4);
    await expect(rows.first()).toHaveAttribute(
      'href',
      'vscode://file/C:/repo/src/orders/book.rs:40',
    );
    await expect(rows.first()).toHaveAttribute('data-selected', 'true');
    await expect(rows.nth(2)).toHaveAttribute('data-selected', 'false');
    await expect(rows.nth(1)).toHaveAttribute('data-nested', 'true');
    await expect(rows.first()).toHaveAccessibleName(
      'OrderBook, a class at lines 40 to 120, selected by 2 work orders. Opens in the editor at its line.',
    );
  });

  test('says the history worker is off instead of promising a history', async ({ page }) => {
    const fixture = smallFixture(Date.now());
    const off: Fixture = {
      ...fixture,
      details: new Map([[3, fileDetail(3, 'src/lib.rs', { history_reader: 'off' })]]),
    };
    record = await openMap(page, off);
    await searchAndPick(page, 'lib.rs');
    await filePanel(page)
      .getByRole('button', { name: /^Commits behind its lines/ })
      .click();
    await expect(filePanel(page).getByTestId('panel-history')).toContainText(
      "Cortex reads the commits behind a file's lines only when history.why or history.seeds is on, and both are off, so no file has them. To read them, run cortex config set history.why true.",
    );
  });

  test("says a generated file's commits are not counted, never 0", async ({ page }) => {
    const fixture = smallFixture(Date.now());
    const readme = fixture.graph.files[0];
    if (readme === undefined) throw new Error('the fixture has no first file');
    const lock: FixtureFile = { ...readme, commits: 0, commits_counted: false };
    const files = [lock, ...fixture.graph.files.slice(1)];
    record = await openMap(page, { ...fixture, graph: { ...fixture.graph, files } });
    await searchAndPick(page, 'README');
    const commits = filePanel(page).locator('dd').nth(2);
    await expect(commits).toHaveText('Not counted');
    await expect(commits).toHaveAccessibleDescription(
      'Cortex does not count commits of generated or vendored files.',
    );
  });
});

test.describe('the quiet refresh', () => {
  test("reads the map's numbers again when a live frame brings a new position, keeping the file selected", async ({
    page,
  }) => {
    const nowMs = Date.now();
    const graphs: string[] = [];
    page.on('request', (request) => {
      if (request.url().endsWith('/api/graph')) graphs.push(request.url());
    });
    record = await openMap(page, smallFixture(nowMs));
    await searchAndPick(page, 'book');
    expect(graphs).toHaveLength(1);
    const live = liveChannel(page);
    live.push({
      version: 1,
      sessions: [liveSession('3f2a9c1e', AGENTS.claude, [[4, 'read']], nowMs)],
    });
    live.push({
      version: 1,
      sessions: [
        liveSession(
          '3f2a9c1e',
          AGENTS.claude,
          [
            [5, 'edit'],
            [4, 'read'],
          ],
          nowMs + 1000,
        ),
      ],
    });
    await expect.poll(() => graphs.length, { timeout: 10_000 }).toBe(2);
    await expect(filePanel(page).locator('#panel-title')).toHaveText('book.rs');
    await expect(page.getByTestId('overview-read-at')).toContainText(
      "by this computer's clock, and read again while agents work.",
    );
  });

  test('keeps the file panel open on its file when a file added before it moves its id', async ({
    page,
  }) => {
    const nowMs = Date.now();
    const fixture = smallFixture(nowMs);
    record = await openMap(page, fixture);
    await searchAndPick(page, 'book');
    const panel = filePanel(page);
    await expect(panel.getByTestId('fact-8')).toBeVisible();
    // The daemon now lists src/billing/aaa.rs first among the files after README.md: every later
    // id moves up by one, book.rs from 4 to 5.
    const [readme, ...rest] = fixture.graph.files;
    if (readme === undefined || rest[0] === undefined) throw new Error('the ten files are there');
    const added: FixtureFile = { ...rest[0], path: 'src/billing/aaa.rs' };
    const files = [readme, added, ...rest].map((file, id) => ({ ...file, id }));
    await page.route('**/api/graph', (route) =>
      route.fulfill({ json: { version: 1, files, links: [] } }),
    );
    const book = fixture.details.get(4);
    if (book === undefined || book === null || typeof book !== 'object' || Array.isArray(book)) {
      throw new Error('book.rs has a detail');
    }
    await page.route('**/api/file/5', (route) => route.fulfill({ json: { ...book, id: 5 } }));
    const live = liveChannel(page);
    live.push({
      version: 1,
      sessions: [liveSession('3f2a9c1e', AGENTS.claude, [[3, 'read']], nowMs)],
    });
    live.push({
      version: 1,
      sessions: [liveSession('3f2a9c1e', AGENTS.claude, [[2, 'read']], nowMs + 1000)],
    });
    await page.waitForRequest('**/api/file/5', { timeout: 10_000 });
    await expect(panel.locator('#panel-title')).toHaveText('book.rs');
    await expect(panel.getByTestId('fact-8')).toBeVisible();
    await expect(panel).not.toContainText('Could not load this');
  });
});
