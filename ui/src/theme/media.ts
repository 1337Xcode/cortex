// React hooks for the system settings the page follows: the colour scheme, reduced motion and a
// narrow viewport. Each subscribes to its media query and re-renders the component on a change.
import { useSyncExternalStore } from 'react';
import { CHROME, type ThemeName } from './tokens';

/** Matches when the system asks for a dark colour scheme. */
export const DARK_QUERY = '(prefers-color-scheme: dark)';

/** Matches a viewport narrower than `CHROME.narrowBelow`, where the chrome compacts. */
const NARROW_QUERY = `(max-width: ${CHROME.narrowBelow - 1}px)`;

/** Matches when the system asks for reduced motion. */
export const REDUCED_MOTION_QUERY = '(prefers-reduced-motion: reduce)';

function useMediaQuery(query: string): boolean {
  return useSyncExternalStore(
    (onChange) => {
      const list = window.matchMedia(query);
      list.addEventListener('change', onChange);
      return () => {
        list.removeEventListener('change', onChange);
      };
    },
    () => window.matchMedia(query).matches,
  );
}

/** The theme that matches the system's colour scheme. */
export function useThemeName(): ThemeName {
  return useMediaQuery(DARK_QUERY) ? 'dark' : 'light';
}

/** Whether the viewport is narrow enough for the compact layout. */
export function useIsNarrow(): boolean {
  return useMediaQuery(NARROW_QUERY);
}

/** Whether the system asks for reduced motion. */
export function useReducedMotion(): boolean {
  return useMediaQuery(REDUCED_MOTION_QUERY);
}
