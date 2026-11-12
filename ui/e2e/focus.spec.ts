// Where the keyboard's focus and keys go: arrow keys on a search result move between results and
// leave the map still; Escape after a pointer pick returns focus to the search box, not to the
// result that went away; and the side panel's folder button opens a folder the map draws
// collapsed, the keyboard's way into it.
import { expect, test, type Page } from '@playwright/test';
import { levelOfDetail, HubState } from '../src/layout/lod';
import { buildScene } from '../src/map/scene';
import { CAMERA, DARK_COLORS } from '../src/theme/tokens';
import { largeFixture, smallFixture } from './fixtures';
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
  type Region,
} from './helpers';

const VIEWPORT = { width: 1440, height: 900 };

let record: PageRecord = { errors: [], foreign: [] };

test.afterEach(() => {
  expect(record.errors, 'console errors').toEqual([]);
  expect(record.foreign, 'requests to other origins').toEqual([]);
});

/** The zoom level the controls show, read back from its percentage. */
async function shownZoom(page: Page): Promise<number> {
  const text = (await page.locator('output.zoom-value').textContent()) ?? '';
  return Math.log2(Number(text.replace('%', '')) / 100);
}

test.describe('ten-file repository', () => {
  test.beforeEach(async ({ page }) => {
    record = await openMap(page, smallFixture(Date.now()));
  });

  test('arrow keys on a search result move between results and leave the map still', async ({
    page,
  }) => {
    await page.keyboard.press('/');
    await page.keyboard.type('rs');
    const results = page.getByRole('list', { name: 'Search results' });
    await expect(results.getByRole('button').nth(1)).toBeVisible();
    // The canvas beside the results list, which the list never covers.
    const top = await headerBottom(page);
    const list = (await results.boundingBox()) ?? { x: 0, y: 0, width: 0, height: 0 };
    const beside: Region =
      list.x > VIEWPORT.width - list.x - list.width
        ? { x: 0, y: top, width: Math.floor(list.x) - 8, height: VIEWPORT.height - top }
        : {
            x: Math.ceil(list.x + list.width) + 8,
            y: top,
            width: VIEWPORT.width - Math.ceil(list.x + list.width) - 8,
            height: VIEWPORT.height - top,
          };
    const ink = { background: DARK_COLORS.background, threshold: 60 };
    const before = await inkIn(page, beside, ink);
    expect(before.count).toBeGreaterThan(200);
    // A pan of one step would move every mark 80 px or more; the map must not move at all. It is
    // read after each key, since a pan down and a pan up would cancel out.
    const expectStill = async (): Promise<void> => {
      await nextFrames(page);
      const now = await inkIn(page, beside, ink);
      expect(now.box).toEqual(before.box);
      expect(now.count).toBe(before.count);
    };
    await page.keyboard.press('ArrowDown');
    await expect(results.getByRole('button').first()).toBeFocused();
    await expectStill();
    await page.keyboard.press('ArrowDown');
    await expect(results.getByRole('button').nth(1)).toBeFocused();
    await expectStill();
    await page.keyboard.press('ArrowUp');
    await expect(results.getByRole('button').first()).toBeFocused();
    await expectStill();
  });

  test('Escape after a pointer pick returns focus to the search box', async ({ page }) => {
    const box = page.getByRole('searchbox', { name: 'Search files and symbols' });
    await box.click();
    await page.keyboard.type('payment');
    const result = page.getByRole('list', { name: 'Search results' }).getByRole('button').first();
    await result.click();
    await expect(filePanel(page)).toBeVisible();
    // The result the pointer picked is gone with its list.
    await expect(result).toBeHidden();
    await page.keyboard.press('Escape');
    await expect(filePanel(page)).toHaveCount(0);
    await expect(box).toBeFocused();
  });
});

test.describe('two-thousand-file repository', () => {
  test("the breadcrumb's last folder opens a folder the map draws collapsed, filling the view", async ({
    page,
  }) => {
    const fixture = largeFixture(2_000);
    record = await openMap(page, fixture);
    const { layout } = buildScene(snapshotOf(fixture));
    const fitted = levelOfDetail(layout, await shownZoom(page));
    // The first file whose folder is collapsed at Fit and large enough to be framed below the
    // camera's zoom limit (32 px a unit, so a radius above 12.5).
    const id = layout.fileHub.findIndex(
      (hub) => hub > 0 && fitted.state[hub] !== HubState.Open && (layout.hubRadius[hub] ?? 0) > 20,
    );
    expect(id).toBeGreaterThanOrEqual(0);
    const hub = layout.fileHub[id] ?? 0;
    const path = fixture.graph.files[id]?.path ?? '';
    await searchAndPick(page, path);
    await expect(page.locator('#panel-title')).toBeFocused();
    const folder = layout.hubPath[hub] ?? '';
    const crumb = filePanel(page).getByRole('button', { name: `Show the folder ${folder}/` });
    // The breadcrumb comes before the title, so Shift+Tab walks back to it.
    for (
      let i = 0;
      i < 20 && !(await crumb.evaluate((e) => e === document.activeElement));
      i += 1
    ) {
      await page.keyboard.press('Shift+Tab');
    }
    await expect(crumb).toBeFocused();
    await page.keyboard.press('Enter');
    await nextFrames(page);
    const zoom = await shownZoom(page);
    expect(levelOfDetail(layout, zoom).state[hub]).toBe(HubState.Open);
    // The folder's circle fills the area the top bar and the panels leave, less the fit padding.
    const insets = await chromeInsets(page, true);
    const width = VIEWPORT.width - insets.left - insets.right;
    const room = Math.min(width, VIEWPORT.height - insets.top) / 2 - CAMERA.fitPaddingPx;
    expect(((layout.hubRadius[hub] ?? 0) * 2 ** zoom) / room).toBeCloseTo(1, 1);
    // The selection stays, and focus stays on the button.
    await expect(page.getByTestId('map-status')).toContainText(path);
    await expect(crumb).toBeFocused();
  });
});
