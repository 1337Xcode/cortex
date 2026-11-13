// A panel section that collapses to its title and count: the title is a button with
// `aria-expanded`, and the body is rendered only while open. The panels open the sections that
// answer the most common questions and leave the rest closed until asked. A section with a letter
// (see `SECTION_KEYS`) shows it beside its title and opens on that key from anywhere on the page
// outside a text field, taking the focus and scrolling to its top. A section's body may end with a
// footnote: one line on how to read it, cut to the panel and shown whole on hover and focus, so the
// answer comes first and its method never pushes the next section down.
import { ChevronRight } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
  type RefObject,
} from 'react';
import { ClippedText } from '../components/Clipped';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../components/ui/collapsible';
import { isKeyField, SECTION_KEYS, sectionFor, type SectionKeyName } from '../controls/shortcuts';
import { formatCount } from '../text/format';

/**
 * Opens the section on its letter: calls `open`, then, once the section has drawn open, focuses
 * `toggle` and scrolls `section` to the top of its panel.
 */
function useSectionKey(
  name: SectionKeyName | undefined,
  open: () => void,
  toggle: RefObject<HTMLButtonElement | null>,
  section: RefObject<HTMLElement | null>,
): void {
  const [request, setRequest] = useState(0);
  useEffect(() => {
    if (name === undefined) return undefined;
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      const { key, ctrlKey, altKey, metaKey } = event;
      const isInField = isKeyField(event.target);
      if (sectionFor({ key, ctrlKey, altKey, metaKey, isInField }) !== name) return;
      event.preventDefault();
      open();
      setRequest((count) => count + 1);
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, [name, open]);
  useEffect(() => {
    if (request === 0) return;
    toggle.current?.focus({ preventScroll: true });
    section.current?.scrollIntoView({ block: 'start' });
  }, [request, toggle, section]);
}

/** One line on how to read a section, at the end of its body: whole on hover and focus. */
export function Footnote({ text }: { text: string }): ReactElement {
  return (
    <p className="section-note section-footnote">
      <ClippedText text={text} kind="words" />
    </p>
  );
}

/** A collapsible section with its title, the count of what it holds, and its body. */
export function Section({
  title,
  count,
  isOpenAtFirst,
  openWhile,
  shortcut,
  testId,
  children,
}: {
  title: string;
  /**
   * How many items the section holds, or what it shows of them when the list is cut ("top 25 of
   * 2,194"); null while that is not known yet.
   */
  count: number | string | null;
  isOpenAtFirst: boolean;
  /**
   * When set, the section opens as this turns true and closes as it turns false, until the person
   * opens or closes it; from then on it keeps their choice.
   */
  openWhile?: boolean;
  /** The section's letter, when it has one. */
  shortcut?: SectionKeyName;
  testId?: string;
  children: ReactNode;
}): ReactElement {
  const [isOpen, setIsOpen] = useState(isOpenAtFirst);
  const [hasChosen, setHasChosen] = useState(false);
  const [followed, setFollowed] = useState(openWhile);
  const toggleRef = useRef<HTMLButtonElement>(null);
  const sectionRef = useRef<HTMLElement>(null);
  // Set while drawing, as React advises for state that follows a prop: no effect, no extra frame.
  if (openWhile !== followed) {
    setFollowed(openWhile);
    if (openWhile !== undefined && !hasChosen) setIsOpen(openWhile);
  }
  const headingId = useId();
  const choose = (open: boolean): void => {
    setHasChosen(true);
    setIsOpen(open);
  };
  const openOnKey = useCallback(() => {
    setHasChosen(true);
    setIsOpen(true);
  }, []);
  useSectionKey(shortcut, openOnKey, toggleRef, sectionRef);
  const letter = shortcut === undefined ? null : SECTION_KEYS[shortcut].toUpperCase();
  return (
    <Collapsible open={isOpen} onOpenChange={choose} asChild>
      <section
        className="section"
        aria-labelledby={headingId}
        data-testid={testId}
        ref={sectionRef}
      >
        <h3 className="section-heading" id={headingId}>
          <CollapsibleTrigger
            className="section-toggle"
            ref={toggleRef}
            aria-keyshortcuts={letter ?? undefined}
          >
            <span className="section-name">
              <span className="section-title">{title}</span>
              {letter === null ? null : (
                <kbd className="key-hint section-key" aria-hidden="true">
                  {letter}
                </kbd>
              )}
            </span>
            {count === null ? null : (
              <span className="section-count">
                {typeof count === 'number' ? formatCount(count) : count}
              </span>
            )}
            <ChevronRight className="chevron" aria-hidden="true" data-open={isOpen} />
          </CollapsibleTrigger>
        </h3>
        <CollapsibleContent className="section-body">{children}</CollapsibleContent>
      </section>
    </Collapsible>
  );
}
