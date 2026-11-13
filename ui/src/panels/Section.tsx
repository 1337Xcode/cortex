// A panel section that collapses to its title and count: the title is a button with
// `aria-expanded`, and the body is rendered only while open. The panels open the sections that
// answer the most common questions and leave the rest closed until asked.
import { ChevronRight } from 'lucide-react';
import { useId, useState, type ReactElement, type ReactNode } from 'react';
import { Collapsible, CollapsibleContent, CollapsibleTrigger } from '../components/ui/collapsible';
import { formatCount } from '../text/format';

/** A collapsible section with its title, the count of what it holds, and its body. */
export function Section({
  title,
  count,
  isOpenAtFirst,
  openWhile,
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
  testId?: string;
  children: ReactNode;
}): ReactElement {
  const [isOpen, setIsOpen] = useState(isOpenAtFirst);
  const [hasChosen, setHasChosen] = useState(false);
  const [followed, setFollowed] = useState(openWhile);
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
  return (
    <Collapsible open={isOpen} onOpenChange={choose} asChild>
      <section className="section" aria-labelledby={headingId} data-testid={testId}>
        <h3 className="section-heading" id={headingId}>
          <CollapsibleTrigger className="section-toggle">
            <span className="section-title">{title}</span>
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
