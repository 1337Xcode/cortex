// How much of the canvas the page's chrome covers: the top bar always; on a wide screen the
// overview panel on the left and, when a file is selected, its panel on the right; on a narrow
// screen the bottom sheet while one is open. The camera centres the graph and the flown-to file
// in what is left.
import { useLayoutEffect, useMemo, useState, type RefObject } from 'react';
import { LAYOUT, SIZE } from '../theme/tokens';
import type { Insets } from './camera';

/** The page's chrome as it stands. */
export interface Chrome {
  /** The top bar. */
  readonly header: RefObject<HTMLElement | null>;
  /** The overview panel, on a wide screen. */
  readonly overview: RefObject<HTMLElement | null>;
  readonly isNarrow: boolean;
  /** Whether a file's panel is open (on the right, or as the sheet on a narrow screen). */
  readonly isFilePanelOpen: boolean;
  /** Whether the overview is open as the sheet on a narrow screen. */
  readonly isOverviewSheetOpen: boolean;
}

/** The edges the chrome's elements reach, in pixels. */
interface Edges {
  readonly headerBottom: number;
  readonly overviewRight: number;
  readonly viewportHeight: number;
}

function measureEdges(chrome: Chrome): Edges {
  const header = chrome.header.current?.getBoundingClientRect();
  const overview = chrome.overview.current?.getBoundingClientRect();
  return {
    headerBottom: Math.ceil(header?.bottom ?? 0),
    overviewRight: chrome.isNarrow ? 0 : Math.ceil(overview?.right ?? 0),
    viewportHeight: window.innerHeight,
  };
}

/** The insets for the current chrome. */
export function useChromeInsets(chrome: Chrome): Insets {
  const [edges, setEdges] = useState<Edges>({
    headerBottom: 0,
    overviewRight: 0,
    viewportHeight: 0,
  });
  useLayoutEffect(() => {
    const measure = (): void => {
      const next = measureEdges(chrome);
      setEdges((previous) =>
        previous.headerBottom === next.headerBottom &&
        previous.overviewRight === next.overviewRight &&
        previous.viewportHeight === next.viewportHeight
          ? previous
          : next,
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    for (const element of [chrome.header.current, chrome.overview.current]) {
      if (element !== null) observer.observe(element);
    }
    window.addEventListener('resize', measure);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [chrome]);
  const { isNarrow, isFilePanelOpen, isOverviewSheetOpen } = chrome;
  const isSheetOpen = isNarrow && (isFilePanelOpen || isOverviewSheetOpen);
  return useMemo(
    () => ({
      top: edges.headerBottom,
      right: isFilePanelOpen && !isNarrow ? SIZE.panel : 0,
      bottom: isSheetOpen ? Math.round(edges.viewportHeight * LAYOUT.sheetHeightShare) : 0,
      left: edges.overviewRight,
    }),
    [edges, isFilePanelOpen, isNarrow, isSheetOpen],
  );
}
