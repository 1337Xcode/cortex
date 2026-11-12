// How much of the canvas the page's chrome covers: the top bar always, the side panel on the right
// when a file is selected, or the bottom sheet on a narrow screen. The camera centres the graph
// and the flown-to file in what is left.
import { useLayoutEffect, useMemo, useState, type RefObject } from 'react';
import { CHROME } from '../theme/tokens';
import type { Insets } from './camera';

/** The insets for the current chrome; `header` is the top bar. */
export function useChromeInsets(
  header: RefObject<HTMLElement | null>,
  isPanelOpen: boolean,
  isNarrow: boolean,
): Insets {
  const [edges, setEdges] = useState({ headerBottom: 0, viewportHeight: 0 });
  useLayoutEffect(() => {
    const element = header.current;
    if (element === null) return undefined;
    const measure = (): void => {
      const headerBottom = Math.ceil(element.getBoundingClientRect().bottom);
      const viewportHeight = window.innerHeight;
      setEdges((previous) =>
        previous.headerBottom === headerBottom && previous.viewportHeight === viewportHeight
          ? previous
          : { headerBottom, viewportHeight },
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    window.addEventListener('resize', measure);
    return () => {
      observer.disconnect();
      window.removeEventListener('resize', measure);
    };
  }, [header]);
  return useMemo(
    () => ({
      top: edges.headerBottom,
      right: isPanelOpen && !isNarrow ? CHROME.panelWidth : 0,
      bottom:
        isPanelOpen && isNarrow ? Math.round(edges.viewportHeight * CHROME.sheetHeightShare) : 0,
      left: 0,
    }),
    [edges, isPanelOpen, isNarrow],
  );
}
