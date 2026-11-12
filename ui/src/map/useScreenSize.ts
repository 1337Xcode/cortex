// Tracks an element's size in CSS pixels, so the camera can fit the graph and the labels know the
// screen they must fit on; and the box of a control over the canvas, so labels stay off it.
import { useLayoutEffect, useState, type RefObject } from 'react';
import type { ScreenBox, ScreenSize } from './labels';

/** The element's current size; zero by zero until it is first measured. */
export function useScreenSize(ref: RefObject<HTMLElement | null>): ScreenSize {
  const [size, setSize] = useState<ScreenSize>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null) return undefined;
    const measure = (): void => {
      const width = element.clientWidth;
      const height = element.clientHeight;
      setSize((previous) =>
        previous.width === width && previous.height === height ? previous : { width, height },
      );
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [ref]);
  return size;
}

/** The element's box relative to its offset parent; empty until it is first measured. */
export function useElementBox(ref: RefObject<HTMLElement | null>): ScreenBox {
  const [box, setBox] = useState<ScreenBox>({ left: 0, top: 0, width: 0, height: 0 });
  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null) return undefined;
    const measure = (): void => {
      const next = {
        left: element.offsetLeft,
        top: element.offsetTop,
        width: element.offsetWidth,
        height: element.offsetHeight,
      };
      setBox((previous) =>
        previous.left === next.left &&
        previous.top === next.top &&
        previous.width === next.width &&
        previous.height === next.height
          ? previous
          : next,
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
  }, [ref]);
  return box;
}
