// One row of a panel's list: a button that flies to a file and selects it, with an optional mark
// before the name, the name cut to the row (whole on hover and focus) and the row's number at its
// end in tabular figures. A row may name a pair of files instead, or a few words and a file (a
// live session's row), each file cut so its end shows. Its tooltip says the whole row in one
// sentence.
import type { ReactElement, ReactNode } from 'react';
import { ClippedLabel, ClippedLeadPath, ClippedPair } from '../components/Clipped';
import { Button } from '../components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../components/ui/tooltip';
import { useMapStore } from '../state/store';
import type { ClipKind } from '../text/clip';

/**
 * What a row shows: a name; two files named together ("a and b"); or a few words then a file
 * ("Codex 9b1c… reading invoice.rs"). Files go by the names the map gives them.
 */
export type RowName =
  | string
  | { readonly first: string; readonly second: string }
  | { readonly lead: string; readonly path: string };

/** The row's name, cut to the row by the rule of its shape. */
function RowNameText({ name, kind }: { name: RowName; kind: ClipKind }): ReactElement {
  if (typeof name === 'string') {
    return <ClippedLabel text={name} kind={kind} className="file-row-name" />;
  }
  if ('lead' in name) {
    return <ClippedLeadPath lead={name.lead} path={name.path} className="file-row-name" />;
  }
  return <ClippedPair first={name.first} second={name.second} className="file-row-name" />;
}

/** A row that selects file `file` and flies to it. */
export function FileRow({
  file,
  name,
  nameKind = 'path',
  measure,
  sentence,
  lead = null,
}: {
  file: number;
  /** What the row shows. */
  name: RowName;
  nameKind?: ClipKind;
  /** The row's number with its unit ("12 commits"), or null for none. */
  measure: string | null;
  /** The whole row as a sentence: the tooltip and the button's accessible name. */
  sentence: string;
  /** A mark or icon drawn before the name, hidden from screen readers (the sentence says it). */
  lead?: ReactNode;
}): ReactElement {
  const selectFile = useMapStore((state) => state.selectFile);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="row"
          className="file-row gap-md"
          aria-label={sentence}
          onClick={() => {
            selectFile(file, { fly: true });
          }}
        >
          <span className="file-row-label">
            {lead}
            <RowNameText name={name} kind={nameKind} />
          </span>
          {measure === null ? null : <span className="file-row-measure">{measure}</span>}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{sentence}</TooltipContent>
    </Tooltip>
  );
}
