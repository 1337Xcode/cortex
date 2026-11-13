// The warnings Cortex gave agents after their edits, open first, then resolved, five rows then
// "Show N more". Each row says what the warning said in words by its kind ("book.rs usually
// changes with mod.rs", its files named as the map names them), then what became of it, its
// strength and its date; its whole sentence (with the files' whole paths, what the strength
// measures for that kind, what its state means) is its accessible name and shows on hover and
// focus. How many are open and resolved is its footnote. A row whose named file the snapshot on
// screen lists, at the path the
// daemon gave, is a button that flies to it; a file it does not hold yet is named by its path, and
// the map is asked to read its data again.
// Closed at first, opened on its letter, asked for when it opens, and read again quietly with the
// rest of the map.
import { useCallback, useEffect, type ReactElement } from 'react';
import { fetchWarnings, MAX_WARNINGS, type GivenWarning } from '../api/warnings';
import { fileIcon } from '../icons/fileIcons';
import { IconSlot } from '../icons/IconSlot';
import { isOutOfStep, listedId, nameFile, namedPath, type NamedFile } from '../map/namedFile';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { countOf, formatCount } from '../text/format';
import { namedFileNote, namedFileSentence, shownName } from '../text/namedFileWords';
import { namesOfFiles, type PathNames } from '../text/pathNames';
import { openFirst, warningLine, warningMeta, warningSentence } from '../text/warningWords';
import { LoadedBody } from './LoadedBody';
import { RowList } from './RowList';
import { Footnote, Section } from './Section';
import { StackRow } from './StackRow';

/** A warning with its two files as the snapshot on screen holds them. */
interface NamedWarning {
  readonly warning: GivenWarning;
  readonly index: number;
  readonly source: NamedFile;
  readonly target: NamedFile;
}

/** The notes a warning's row adds for its files named by path, each said once. */
function fileNotes(named: NamedWarning): {
  readonly parts: string[];
  readonly sentences: string[];
} {
  const files = [named.source, named.target];
  const parts = [...new Set(files.map(namedFileNote).filter((note) => note !== null))];
  const sentences = files.map(namedFileSentence).filter((sentence) => sentence !== null);
  return { parts, sentences: [...new Set(sentences)] };
}

/** One warning's row: a button that flies to the named file, or plain text without one. */
function WarningRow({ named, names }: { named: NamedWarning; names: PathNames }): ReactElement {
  const { warning, source, target } = named;
  const [sourceName, targetName] = [shownName(source, names), shownName(target, names)];
  const targetPath = namedPath(target);
  const notes = fileNotes(named);
  return (
    <StackRow
      file={listedId(target)}
      lead={<IconSlot icon={targetPath === null ? null : fileIcon(targetPath)} />}
      what={warningLine(warning.kind, sourceName, targetName)}
      meta={[...warningMeta(warning), ...notes.parts]}
      sentence={[warningSentence(warning, namedPath(source), targetPath), ...notes.sentences].join(
        ' ',
      )}
      state={warning.state}
    />
  );
}

/** The warnings once read, against the snapshot of `scene`. */
function WarningList({
  warnings,
  scene,
}: {
  warnings: readonly GivenWarning[];
  scene: Scene;
}): ReactElement {
  const { files } = scene.snapshot;
  const noteOutOfStep = useMapStore((state) => state.noteOutOfStep);
  const isBehind = warnings.some(
    (warning) =>
      isOutOfStep(files, warning.source, warning.source_path) ||
      isOutOfStep(files, warning.target, warning.target_path),
  );
  useEffect(() => {
    if (isBehind) noteOutOfStep();
  }, [isBehind, noteOutOfStep, files]);
  if (warnings.length === 0) {
    return (
      <p className="panel-empty">
        Cortex has given no warning here yet. It warns an agent after an edit when a file that
        usually changes with it, its tests, or a caller of a changed signature is left untouched.
      </p>
    );
  }
  // An id counts only where the snapshot holds the same path: ids move when files are added.
  const items: NamedWarning[] = openFirst(warnings).map((warning, index) => ({
    warning,
    index,
    source: nameFile(files, warning.source, warning.source_path),
    target: nameFile(files, warning.target, warning.target_path),
  }));
  const names = namesOfFiles(files);
  const open = items.filter(({ warning }) => warning.state === 'open').length;
  return (
    <>
      <RowList
        items={items}
        keyOf={({ index }) => index}
        label="Warnings, open first"
        renderRow={(named) => <WarningRow named={named} names={names} />}
      />
      <Footnote
        text={`${countOf(open, 'warning')} open and ${formatCount(items.length - open)} resolved, among the ${formatCount(MAX_WARNINGS)} newest Cortex gave; open first.`}
      />
    </>
  );
}

/** The overview's warnings section: its count is the warnings read, once they are. */
export function WarningsSection({ scene }: { scene: Scene }): ReactElement {
  const warnings = useMapStore((state) => state.warnings);
  const loadWarnings = useMapStore((state) => state.loadWarnings);
  const load = useCallback(() => {
    void loadWarnings(fetchWarnings);
  }, [loadWarnings]);
  return (
    <Section
      title="Warnings"
      count={warnings.status === 'ready' ? warnings.value.warnings.length : null}
      isOpenAtFirst={false}
      shortcut="warnings"
      testId="overview-warnings"
    >
      <LoadedBody state={warnings} load={load} what="the warnings" testId="warnings-error">
        {(value) => <WarningList warnings={value.warnings} scene={scene} />}
      </LoadedBody>
    </Section>
  );
}
