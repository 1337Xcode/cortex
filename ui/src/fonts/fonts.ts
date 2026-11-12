// The map's two fonts, Geist and Geist Mono (SIL Open Font License 1.1), served from the map's own
// origin: only their Latin variable-weight files are bundled, so no font request leaves the page.
// They are registered through the FontFace API, and `useFontsSettled` turns true once the browser
// has loaded both (or failed to). The canvas builds its first label atlas only after that, so no
// label is ever drawn in a fallback font and then redrawn.
import { useSyncExternalStore } from 'react';
import geistMonoUrl from '@fontsource-variable/geist-mono/files/geist-mono-latin-wght-normal.woff2?url';
import geistUrl from '@fontsource-variable/geist/files/geist-latin-wght-normal.woff2?url';
import { FONT_NAME, TYPE, WEIGHT } from '../theme/tokens';

/** The characters the Latin files cover, as the font package declares them. */
const LATIN_RANGE =
  'U+0000-00FF,U+0131,U+0152-0153,U+02BB-02BC,U+02C6,U+02DA,U+02DC,U+0304,U+0308,U+0329,' +
  'U+2000-206F,U+20AC,U+2122,U+2191,U+2193,U+2212,U+2215,U+FEFF,U+FFFD';

/** The weights each variable file holds. */
const WEIGHT_RANGE = '100 900';

/** Where loading the fonts stands. */
type FontStatus = 'loading' | 'ready' | 'failed';

let status: FontStatus = 'loading';
const listeners = new Set<() => void>();

function settle(next: FontStatus): void {
  status = next;
  for (const listener of listeners) listener();
}

/** Registers one family from its bundled file and waits until the browser can draw with it. */
async function loadFamily(family: string, url: string): Promise<void> {
  const face = new FontFace(family, `url(${url}) format('woff2')`, {
    weight: WEIGHT_RANGE,
    unicodeRange: LATIN_RANGE,
    display: 'block',
  });
  document.fonts.add(face);
  await face.load();
  await document.fonts.load(`${WEIGHT.regular} ${TYPE.body.sizePx}px "${family}"`);
}

/**
 * Loads both fonts once; the page calls it at start. When a file cannot be loaded (a build that
 * lost it), the cause is logged as an error and the page goes on in the system's fonts rather than
 * show no map at all.
 */
export async function loadFonts(): Promise<void> {
  try {
    await Promise.all([
      loadFamily(FONT_NAME.sans, geistUrl),
      loadFamily(FONT_NAME.mono, geistMonoUrl),
    ]);
    settle('ready');
  } catch (error) {
    console.error('the map could not load its fonts', error);
    settle('failed');
  }
}

/**
 * Whether loading the fonts has finished, so labels can be measured and drawn; re-renders the
 * component when it does.
 */
export function useFontsSettled(): boolean {
  return useSyncExternalStore(
    (onChange) => {
      listeners.add(onChange);
      return () => {
        listeners.delete(onChange);
      };
    },
    () => status !== 'loading',
  );
}
