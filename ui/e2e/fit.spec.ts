// Nothing spilled, nothing crammed, one family: at 1440x900 and 390x844 in both themes, on the
// ten-file repository with a file selected and every section open, on the cold repository and on
// the overview of 100,000 files, with each view and its legend, with three live sessions and with
// the live view paused, every element is walked and the check fails on
// - a box whose content is wider or taller than the box, unless the box is a declared scroll area
//   (an open overlay, such as the search results, is checked on its own, apart from its anchor);
// - visible text drawn below 12 px;
// - visible text whose font family does not start with Geist, or a Geist font not yet loaded.
// A last check delays the font files and sees that the canvas waits for them before it draws.
import { expect, test, type Page } from '@playwright/test';
import { TYPE } from '../src/theme/tokens';
import { coldFixture, largeFixture, smallFixture, threeSessions, type Fixture } from './fixtures';
import { filePanel, nextFrames, openMap, searchAndPick } from './helpers';
import { liveChannel, MAP_URL, serveFixture } from './serve';

const SIZES = [
  { name: '1440x900', width: 1440, height: 900 },
  { name: '390x844', width: 390, height: 844 },
] as const;
const SCHEMES = ['dark', 'light'] as const;

/** What the walk found wrong, one line per element. */
async function findProblems(page: Page, minSizePx: number): Promise<string[]> {
  await nextFrames(page);
  return page.evaluate((minSize) => {
    const problems: string[] = [];
    const nameOf = (element: Element): string => {
      const classes = [...element.classList].slice(0, 3).join('.');
      const text = element.textContent.trim().slice(0, 40);
      return `${element.tagName.toLowerCase()}${classes === '' ? '' : `.${classes}`} "${text}"`;
    };
    const isHidden = (element: Element): boolean =>
      element.closest('.visually-hidden, [hidden], [aria-hidden="true"]') !== null ||
      !element.checkVisibility({ checkOpacity: true, checkVisibilityCSS: true });
    const spillOf = (element: HTMLElement): string | null => {
      const isWider = element.scrollWidth > element.clientWidth;
      const isTaller = element.scrollHeight > element.clientHeight;
      if (element.dataset['scrollArea'] === 'true' || element.clientWidth === 0) return null;
      if (!isWider && !isTaller) return null;
      const sizes = `${element.scrollWidth}x${element.scrollHeight} in ${element.clientWidth}x${element.clientHeight}`;
      return `spills (${sizes}): ${nameOf(element)}`;
    };
    // Overlays float over the page: their anchors are measured without them, and they on their own.
    const overlays = [...document.querySelectorAll<HTMLElement>('[data-overlay="true"]')];
    const shown = overlays.map((overlay) => overlay.style.display);
    for (const overlay of overlays) overlay.style.display = 'none';
    const anchored = [...document.querySelectorAll('body *')].map((element) =>
      element instanceof HTMLElement && !isHidden(element) ? spillOf(element) : null,
    );
    for (const [index, overlay] of overlays.entries()) overlay.style.display = shown[index] ?? '';
    for (const [index, element] of [...document.querySelectorAll('body *')].entries()) {
      if (!(element instanceof HTMLElement) || isHidden(element)) continue;
      const isInOverlay = element.closest('[data-overlay="true"]') !== null;
      const spill = isInOverlay ? spillOf(element) : (anchored[index] ?? null);
      if (spill !== null) problems.push(spill);
      const hasOwnText = [...element.childNodes].some(
        (node) => node.nodeType === Node.TEXT_NODE && (node.textContent ?? '').trim() !== '',
      );
      const isField = element instanceof HTMLInputElement;
      if (!hasOwnText && !isField) continue;
      const style = getComputedStyle(element);
      const size = parseFloat(style.fontSize);
      if (size < minSize) problems.push(`text at ${size}px: ${nameOf(element)}`);
      const family = style.fontFamily.replace(/^["']/, '');
      if (!family.startsWith('Geist'))
        problems.push(`font ${style.fontFamily}: ${nameOf(element)}`);
    }
    // An open overlay lies wholly inside the viewport.
    for (const overlay of overlays) {
      if (isHidden(overlay)) continue;
      const box = overlay.getBoundingClientRect();
      const isInside =
        box.left >= 0 && box.top >= 0 && box.right <= innerWidth && box.bottom <= innerHeight;
      if (!isInside) problems.push(`overlay outside the viewport: ${nameOf(overlay)}`);
    }
    for (const family of ['Geist', 'Geist Mono']) {
      if (!document.fonts.check(`${minSize}px "${family}"`)) problems.push(`${family} not loaded`);
    }
    return problems;
  }, minSizePx);
}

/** Opens every closed section of the file panel. */
async function openSections(page: Page): Promise<void> {
  const closed = filePanel(page).locator('.section-toggle[aria-expanded="false"]');
  while ((await closed.count()) > 0) await closed.first().click();
}

for (const size of SIZES) {
  for (const scheme of SCHEMES) {
    const prefix = `${size.name} ${scheme}`;

    test(`${prefix}: the ten-file map with a file selected and every section open`, async ({
      page,
    }) => {
      await page.setViewportSize(size);
      const record = await openMap(page, smallFixture(Date.now()), scheme, true);
      expect(await findProblems(page, TYPE.small.sizePx), 'first open').toEqual([]);
      await page.getByRole('button', { name: 'Got it' }).click();
      if (size.width < 720) {
        await page.getByRole('button', { name: 'Overview' }).click();
        expect(await findProblems(page, TYPE.small.sizePx), 'overview sheet').toEqual([]);
        await page.getByRole('button', { name: 'Close the overview' }).click();
      }
      await searchAndPick(page, 'book');
      await openSections(page);
      expect(await findProblems(page, TYPE.small.sizePx), 'selected').toEqual([]);
      await page.keyboard.press('Escape');
      // Escape returns focus to the search box; leave it, so L reaches the map.
      await page.keyboard.press('Tab');
      await page.keyboard.press('l');
      expect(await findProblems(page, TYPE.small.sizePx), 'legend').toEqual([]);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: every view with its legend, live sessions and the paused live view`, async ({
      page,
    }) => {
      await page.setViewportSize(size);
      const nowMs = Date.now();
      const fixture: Fixture = { ...smallFixture(nowMs), live: threeSessions(nowMs) };
      const record = await openMap(page, fixture, scheme);
      if (size.width < 720) await page.getByRole('button', { name: 'Overview' }).click();
      await expect(page.getByTestId('overview-live').getByRole('listitem')).toHaveCount(3);
      expect(await findProblems(page, TYPE.small.sizePx), 'live sessions').toEqual([]);
      if (size.width < 720) await page.getByRole('button', { name: 'Close the overview' }).click();
      await page.getByRole('button', { name: 'Legend' }).click();
      for (const [key, title] of [
        ['1', 'Default'],
        ['2', 'Fog'],
        ['3', 'Heat'],
        ['4', 'Coupling'],
        ['5', 'Recent'],
      ] as const) {
        await page.keyboard.press(key);
        await expect(page.getByTestId('legend-view')).toHaveText(`View: ${title}`);
        expect(await findProblems(page, TYPE.small.sizePx), title).toEqual([]);
      }
      await page.getByRole('button', { name: 'Legend' }).click();
      const live = liveChannel(page);
      live.setRefusing(true);
      live.closeAll();
      await expect(page.getByTestId('live-paused')).toBeVisible();
      expect(await findProblems(page, TYPE.small.sizePx), 'paused').toEqual([]);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: the cold repository`, async ({ page }) => {
      await page.setViewportSize(size);
      const record = await openMap(page, coldFixture(), scheme);
      expect(await findProblems(page, TYPE.small.sizePx)).toEqual([]);
      expect(record.errors).toEqual([]);
    });

    test(`${prefix}: the overview of 100,000 files`, async ({ page }) => {
      test.setTimeout(240_000);
      await page.setViewportSize(size);
      const large: Fixture = largeFixture(100_000);
      const record = await openMap(page, large, scheme);
      expect(await findProblems(page, TYPE.small.sizePx)).toEqual([]);
      expect(record.errors).toEqual([]);
    });
  }
}

test('the canvas draws no label before both fonts have loaded', async ({ page }) => {
  await serveFixture(page, smallFixture(Date.now()));
  // Each font file arrives a second and a half late; the snapshot arrives at once.
  await page.route('**/*.woff2', async (route) => {
    await new Promise((resolve) => setTimeout(resolve, 1500));
    await route.fallback();
  });
  // Whether both fonts could draw at the moment the map (its canvas, and so its first label
  // atlas) first appeared in the page, recorded in the page as it happens.
  await page.addInitScript(() => {
    window.localStorage.setItem('cortex-map-intro-dismissed', 'true');
    const observer = new MutationObserver(() => {
      if (document.querySelector('[data-testid="map-status"]') === null) return;
      const families = ['Geist', 'Geist Mono'];
      const loaded = families.map((family) => document.fonts.check(`12px "${family}"`));
      Reflect.set(window, '__fontsAtFirstMap', loaded);
      observer.disconnect();
    });
    observer.observe(document, { childList: true, subtree: true });
  });
  await page.goto(MAP_URL, { waitUntil: 'commit' });
  await expect(page.getByTestId('map-status')).toBeAttached();
  const loaded = await page.evaluate((): unknown => Reflect.get(window, '__fontsAtFirstMap'));
  expect(loaded).toEqual([true, true]);
});
