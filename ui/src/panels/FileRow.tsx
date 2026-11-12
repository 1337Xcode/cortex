// One row of a panel's list: a button that flies to a file and selects it, with the file's name
// cut to the row (whole on hover and focus) and the row's number at its end in tabular figures.
// Its tooltip says the whole row in one sentence.
import type { ReactElement } from 'react';
import { ClippedLabel } from '../components/Clipped';
import { Button } from '../components/ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../components/ui/tooltip';
import { useMapStore } from '../state/store';
import type { ClipKind } from '../text/clip';

/** A row that selects file `file` and flies to it. */
export function FileRow({
  file,
  name,
  nameKind = 'path',
  measure,
  sentence,
}: {
  file: number;
  /** What the row shows: a file name, a path or a pair of names. */
  name: string;
  nameKind?: ClipKind;
  /** The row's number with its unit ("12 commits"), or null for none. */
  measure: string | null;
  /** The whole row as a sentence: the tooltip and the button's accessible name. */
  sentence: string;
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
          <ClippedLabel text={name} kind={nameKind} className="file-row-name" />
          {measure === null ? null : <span className="file-row-measure">{measure}</span>}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{sentence}</TooltipContent>
    </Tooltip>
  );
}
