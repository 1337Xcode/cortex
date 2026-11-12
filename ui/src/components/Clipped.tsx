// Text that may be longer than its box: shown whole when it fits, otherwise cut with an ellipsis at
// a path or word boundary (see text/clip.ts), and shown whole again on hover and on keyboard focus.
// The cut is measured in the element's own font and width and redone when the box resizes, so
// nothing spills out of its box and nothing scrolls sideways. The box's width must come from its
// container (a block, or a flexible item that may shrink), never from the text itself: a box that
// shrank to the cut text would be cut again.
import { useLayoutEffect, useRef, useState, type ReactElement, type RefObject } from 'react';
import { useFontsSettled } from '../fonts/fonts';
import { cn } from '../lib/cn';
import { clipText, type ClipKind } from '../text/clip';
import { fontMeasurer } from '../text/measure';
import { Tooltip, TooltipContent, TooltipTrigger } from './ui/tooltip';

/**
 * A width measured in the canvas can differ from the laid-out width by a fraction of a pixel; text
 * is fitted this much narrower than its box so the rounding never makes it spill.
 */
const SUBPIXEL_SLACK_PX = 1;

/** `text` as it fits the element `ref` points at, cut by the rule of `kind`. */
export function useClippedText(
  ref: RefObject<HTMLElement | null>,
  text: string,
  kind: ClipKind,
): string {
  const [shown, setShown] = useState(text);
  const isSettled = useFontsSettled();
  useLayoutEffect(() => {
    const element = ref.current;
    if (element === null || !isSettled) return undefined;
    const fit = (): void => {
      const width = element.clientWidth - SUBPIXEL_SLACK_PX;
      const font = getComputedStyle(element).font;
      const measure = fontMeasurer();
      setShown(width <= 0 ? text : clipText(text, width, kind, (part) => measure(part, font)));
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(element);
    return () => {
      observer.disconnect();
    };
  }, [ref, text, kind, isSettled]);
  return shown;
}

/**
 * Text that is cut to its box. When it is cut it takes keyboard focus and shows the whole text in
 * a tooltip on hover and focus; whole text needs neither. The tooltip is always there, only kept
 * shut while the text is whole, so the element the cut is measured on is never replaced.
 */
export function ClippedText({
  text,
  kind,
  className,
}: {
  text: string;
  kind: ClipKind;
  className?: string;
}): ReactElement {
  const ref = useRef<HTMLSpanElement>(null);
  const shown = useClippedText(ref, text, kind);
  const isCut = shown !== text;
  return (
    <Tooltip {...(isCut ? {} : { open: false })}>
      <TooltipTrigger asChild>
        <span
          ref={ref}
          className={cn('clip', className)}
          tabIndex={isCut ? 0 : undefined}
          aria-label={isCut ? text : undefined}
        >
          {shown}
        </span>
      </TooltipTrigger>
      <TooltipContent>{text}</TooltipContent>
    </Tooltip>
  );
}

/** Text cut to its box inside a control that already explains itself (a row's tooltip). */
export function ClippedLabel({
  text,
  kind,
  className,
}: {
  text: string;
  kind: ClipKind;
  className?: string;
}): ReactElement {
  const ref = useRef<HTMLSpanElement>(null);
  const shown = useClippedText(ref, text, kind);
  return (
    <span ref={ref} className={cn('clip', className)}>
      {shown}
    </span>
  );
}
