// Opening the real map in the browser as a person does: `cortex map --no-open` prints the address
// (starting the map again if it closed while idle), and the page goes to it. Everything the page
// logs as an error, and every request it makes to another origin, is kept for the check to assert
// empty.
import { expect, type Page } from '@playwright/test';
import { nextFrames, searchAndPick } from '../helpers';
import { mapAddress } from './daemon';
import { readState, type State } from './state';

/** Where the page remembers that the first-open sentence was dismissed. */
const INTRO_KEY = 'cortex-map-intro-dismissed';

/** An opened map: the setup's state, the address and what the page did that a check asserts. */
export interface Opened {
  readonly state: State;
  /** The address `cortex map --no-open` printed. */
  readonly address: string;
  /** Console errors and uncaught exceptions. */
  readonly errors: string[];
  /** Requests to any origin but the map's. */
  readonly foreign: string[];
}

/**
 * Opens the map of the setup's repository and waits for its first frames. The first-open sentence
 * counts as dismissed, so the checks see the map as a person sees it from the second open on;
 * motion is off, as when the system asks for reduced motion.
 */
export async function openRealMap(page: Page): Promise<Opened> {
  const state = readState();
  const address = await mapAddress(state);
  const origin = new URL(address).origin;
  const errors: string[] = [];
  const foreign: string[] = [];
  page.on('console', (message) => {
    if (message.type() === 'error') errors.push(message.text());
  });
  page.on('pageerror', (error) => errors.push(error.message));
  page.on('request', (request) => {
    if (new URL(request.url()).origin !== origin) foreign.push(request.url());
  });
  await page.addInitScript((key) => {
    window.localStorage.setItem(key, 'true');
  }, INTRO_KEY);
  await page.emulateMedia({ colorScheme: 'dark', reducedMotion: 'reduce' });
  await page.goto(address);
  await expect(page.getByTestId('map-status')).toBeAttached();
  await nextFrames(page);
  return { state, address, errors, foreign };
}

/** Asserts that the page logged no error and asked nothing of any origin but the map's. */
export function expectClean(opened: Opened): void {
  expect(opened.errors).toEqual([]);
  expect(opened.foreign).toEqual([]);
}

/**
 * Searches with the keyboard for `text`, picks the first result with Enter and waits until the
 * picked file's panel has the focus.
 */
export async function openFile(page: Page, text: string): Promise<void> {
  await searchAndPick(page, text);
  await expect(page.locator('#panel-title')).toBeFocused();
}

/**
 * Closes the open file's panel with Escape, which gives the focus back to the search box that
 * opened it, still holding the last search; selects that text, types `text` over it and picks the
 * first result with Enter, as a person searching again does. Waits until the picked file's panel
 * has the focus.
 */
export async function searchAgain(page: Page, text: string): Promise<void> {
  await page.keyboard.press('Escape');
  const box = page.getByRole('searchbox', { name: 'Search files and symbols' });
  await expect(box).toBeFocused();
  await page.keyboard.press('ControlOrMeta+A');
  await page.keyboard.type(text);
  await expect(box).toHaveValue(text);
  await expect(
    page.getByRole('list', { name: 'Search results' }).getByRole('button').first(),
  ).toBeVisible();
  await page.keyboard.press('Enter');
  await expect(page.locator('#panel-title')).toBeFocused();
}
