// A list that shows its first five rows and a "Show N more" button for the rest, and a "Show
// fewer" button once they are shown, so a long list never floods its panel.
import { useState, type ReactElement, type ReactNode } from 'react';
import { Button } from '../components/ui/button';
import { formatCount } from '../text/format';
import { LIST } from '../theme/tokens';

/** The first `LIST.rows` of `items` as list rows, and the button that shows the others. */
export function RowList<T>({
  items,
  keyOf,
  renderRow,
  label,
}: {
  items: readonly T[];
  keyOf: (item: T) => string | number;
  renderRow: (item: T) => ReactNode;
  /** What the list holds, for screen readers. */
  label: string;
}): ReactElement {
  const [isAll, setIsAll] = useState(false);
  const hidden = items.length - LIST.rows;
  const shown = isAll || hidden <= 0 ? items : items.slice(0, LIST.rows);
  return (
    <>
      <ul className="row-list" aria-label={label}>
        {shown.map((item) => (
          <li key={keyOf(item)}>{renderRow(item)}</li>
        ))}
      </ul>
      {hidden > 0 ? (
        <Button
          variant="ghost"
          size="compact"
          className="show-more -ml-sm text-muted-foreground hover:text-foreground"
          aria-expanded={isAll}
          onClick={() => {
            setIsAll(!isAll);
          }}
        >
          {isAll ? 'Show fewer' : `Show ${formatCount(hidden)} more`}
        </Button>
      ) : null}
    </>
  );
}
