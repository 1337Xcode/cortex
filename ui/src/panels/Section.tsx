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
  testId,
  children,
}: {
  title: string;
  /** How many items the section holds, or null while that is not known yet. */
  count: number | null;
  isOpenAtFirst: boolean;
  testId?: string;
  children: ReactNode;
}): ReactElement {
  const [isOpen, setIsOpen] = useState(isOpenAtFirst);
  const headingId = useId();
  return (
    <Collapsible open={isOpen} onOpenChange={setIsOpen} asChild>
      <section className="section" aria-labelledby={headingId} data-testid={testId}>
        <h3 className="section-heading" id={headingId}>
          <CollapsibleTrigger className="section-toggle">
            <span className="section-title">{title}</span>
            {count === null ? null : <span className="section-count">{formatCount(count)}</span>}
            <ChevronRight className="chevron" aria-hidden="true" data-open={isOpen} />
          </CollapsibleTrigger>
        </h3>
        <CollapsibleContent className="section-body">{children}</CollapsibleContent>
      </section>
    </Collapsible>
  );
}
