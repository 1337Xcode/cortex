// The collapsible legend: the active view by name, what its scale means with its numbers, and each
// of its tones with how many files show it; the marks every view draws (the suspect ring, the
// recent centre, fact badges, live markers and their trails) with their counts; the folder
// shapes; each link kind with how many links the threshold leaves drawn, and the slider that sets
// it; and the way back to the sentence that says what the picture is. Collapsed by default.
import { ChevronUp } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useState, type ReactElement } from 'react';
import { LINK_KINDS } from '../api/graph';
import { Button } from '../components/ui/button';
import { fileMark, type FileMark } from '../map/fileEncoding';
import { countLinksByKind } from '../map/linkEncoding';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { formatCount } from '../text/format';
import {
  FOLDER_MEANINGS,
  FOLDER_NAMES,
  FOLDER_RULE_SENTENCES,
  LINK_KIND_MEANINGS,
  LINK_KIND_NAMES,
  OVERLAY_MARKS,
} from '../text/vocabulary';
import type { ViewPaint } from '../views/paint';
import {
  BadgeSample,
  CircleSample,
  DiamondSample,
  FolderSample,
  LineSample,
  TrailSample,
} from './legendSamples';
import { isKeyField, shortcutFor } from './shortcuts';

/** Counts the files showing each mark. */
function countMarks(scene: Scene, nowMs: number): Readonly<Record<FileMark, number>> {
  const counts: Record<FileMark, number> = { suspect: 0, recent: 0, unread: 0, plain: 0 };
  for (const file of scene.snapshot.files) counts[fileMark(file, nowMs, scene.hasReads)] += 1;
  return counts;
}

/** One legend entry: its sample, its name and meaning, and its count. */
function Entry(props: {
  testId: string;
  sample: ReactElement;
  name: string;
  meaning: string;
  count: number | null;
}): ReactElement {
  return (
    <li className="legend-entry" data-testid={props.testId}>
      {props.sample}
      <span className="legend-name">{props.name}</span>
      <span className="legend-count">{props.count === null ? '' : formatCount(props.count)}</span>
      {props.meaning === '' ? null : <span className="legend-meaning">{props.meaning}</span>}
    </li>
  );
}

function StrengthSlider(): ReactElement {
  const id = useId();
  const minStrength = useMapStore((state) => state.minStrength);
  const setMinStrength = useMapStore((state) => state.setMinStrength);
  return (
    <div className="legend-slider">
      <label htmlFor={id}>Draw links of strength at least</label>
      <div className="legend-slider-row">
        <input
          id={id}
          type="range"
          min={0}
          max={1}
          step={0.05}
          value={minStrength}
          onChange={(event) => {
            setMinStrength(Number(event.target.value));
          }}
        />
        <output htmlFor={id}>{minStrength.toFixed(2)}</output>
      </div>
    </div>
  );
}

/** The active view's name, scale and tones, each tone with how many files show it. */
function ViewEntries({ view }: { view: ViewPaint }): ReactElement {
  return (
    <>
      <h2 className="legend-heading" data-testid="legend-view">
        {`View: ${view.title}`}
      </h2>
      <p className="legend-scale" data-testid="legend-scale">
        {view.encoding.scale}
        {view.encoding.isTinted ? ` ${FOLDER_RULE_SENTENCES[view.encoding.folderRule]}` : ''}
      </p>
      <ul className="legend-list">
        {view.encoding.tones.map((tone) => (
          <Entry
            key={tone.key}
            testId={`legend-${tone.key}`}
            sample={
              <CircleSample
                role={tone.role}
                isDim={tone.isDim}
                isFilled={view.encoding.isTinted && !tone.isDim}
                mark={tone.mark}
              />
            }
            name={tone.name}
            meaning={tone.meaning}
            count={tone.count}
          />
        ))}
      </ul>
    </>
  );
}

/**
 * The marks drawn over files in every view, with their counts. A view whose tones already show the
 * suspect ring and the recent centre (the default view) does not list them twice.
 */
function MarkEntries(props: {
  scene: Scene;
  nowMs: number;
  view: ViewPaint;
  liveCount: number;
}): ReactElement {
  const { scene, nowMs, view, liveCount } = props;
  const marks = useMemo(() => countMarks(scene, nowMs), [scene, nowMs]);
  const withFacts = useMemo(
    () => scene.snapshot.files.filter((file) => file.facts > 0).length,
    [scene],
  );
  const isShownByTones = view.encoding.tones.some((tone) => tone.mark !== null);
  return (
    <ul className="legend-list">
      {isShownByTones ? null : (
        <>
          <Entry
            testId="legend-mark-suspect"
            sample={<CircleSample role="file" isDim={false} mark="ring" />}
            {...OVERLAY_MARKS.suspect}
            count={marks.suspect}
          />
          <Entry
            testId="legend-mark-recent"
            sample={<CircleSample role="file" isDim={false} mark="dot" />}
            {...OVERLAY_MARKS.recent}
            count={marks.recent}
          />
        </>
      )}
      <Entry
        testId="legend-badge"
        sample={<BadgeSample />}
        {...OVERLAY_MARKS.badge}
        count={withFacts}
      />
      <Entry
        testId="legend-marker"
        sample={<DiamondSample />}
        {...OVERLAY_MARKS.marker}
        count={liveCount}
      />
      <Entry testId="legend-trail" sample={<TrailSample />} {...OVERLAY_MARKS.trail} count={null} />
    </ul>
  );
}

/** The two folder marks, the square counted. */
function FolderEntries({ scene }: { scene: Scene }): ReactElement {
  return (
    <ul className="legend-list">
      <Entry
        testId="legend-folders"
        sample={<FolderSample isCollapsed={false} />}
        name={FOLDER_NAMES.folder}
        meaning={FOLDER_MEANINGS.folder}
        count={scene.layout.hubPath.length - 1}
      />
      <Entry
        testId="legend-collapsed"
        sample={<FolderSample isCollapsed />}
        name={FOLDER_NAMES.collapsed}
        meaning={FOLDER_MEANINGS.collapsed}
        count={null}
      />
    </ul>
  );
}

/** The link kinds, each with how many links of it the threshold leaves drawn. */
function LinkEntries({ scene }: { scene: Scene }): ReactElement {
  const minStrength = useMapStore((state) => state.minStrength);
  const linkCounts = useMemo(
    () => countLinksByKind(scene.snapshot.links, minStrength),
    [scene, minStrength],
  );
  return (
    <ul className="legend-list">
      {LINK_KINDS.map((kind) => (
        <Entry
          key={kind}
          testId={`legend-${kind}`}
          sample={<LineSample kind={kind} />}
          name={LINK_KIND_NAMES[kind]}
          meaning={LINK_KIND_MEANINGS[kind]}
          count={linkCounts[kind]}
        />
      ))}
    </ul>
  );
}

/** What the legend explains: the map of `scene`, coloured by `view`, with `liveCount` sessions marked. */
interface LegendProps {
  readonly scene: Scene;
  readonly nowMs: number;
  readonly view: ViewPaint;
  readonly liveCount: number;
}

function LegendBody(props: LegendProps): ReactElement {
  const setIntroOpen = useMapStore((state) => state.setIntroOpen);
  return (
    <div className="legend-body" data-scroll-area="true">
      <ViewEntries view={props.view} />
      <h2 className="legend-heading">Marks</h2>
      <MarkEntries {...props} />
      <h2 className="legend-heading">Folders</h2>
      <FolderEntries scene={props.scene} />
      <h2 className="legend-heading">Links drawn</h2>
      <LinkEntries scene={props.scene} />
      <StrengthSlider />
      <Button
        variant="link"
        size="inline"
        className="legend-intro"
        onClick={() => {
          setIntroOpen(true);
        }}
      >
        What does this picture show?
      </Button>
    </div>
  );
}

/** Toggles the legend when `L` is pressed outside a text field. */
function useLegendKey(toggle: () => void): void {
  useEffect(() => {
    const onKey = (event: KeyboardEvent): void => {
      if (event.defaultPrevented) return;
      const { key, ctrlKey, altKey, metaKey } = event;
      const isInField = isKeyField(event.target);
      if (shortcutFor({ key, ctrlKey, altKey, metaKey, isInField }) !== 'toggleLegend') return;
      event.preventDefault();
      toggle();
    };
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('keydown', onKey);
    };
  }, [toggle]);
}

/** The legend, with a button that shows and hides it; `L` does the same. */
export function Legend(props: LegendProps): ReactElement {
  const [isOpen, setIsOpen] = useState(false);
  const bodyId = useId();
  const toggle = useCallback(() => {
    setIsOpen((was) => !was);
  }, []);
  useLegendKey(toggle);
  return (
    <section className="legend" aria-label="Legend">
      <Button
        className="legend-toggle"
        aria-expanded={isOpen}
        aria-controls={bodyId}
        aria-keyshortcuts="L"
        title="Show or hide the legend (L)"
        onClick={toggle}
      >
        <ChevronUp className="chevron" aria-hidden="true" data-open={isOpen} />
        <span>Legend</span>
      </Button>
      <div id={bodyId} className="legend-popover" hidden={!isOpen} data-overlay="true">
        {isOpen ? <LegendBody {...props} /> : null}
      </div>
    </section>
  );
}
