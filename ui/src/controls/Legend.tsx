// The collapsible legend: each shape the canvas draws, with how many of each this repository has
// and a line on what it means (shape first, colour confirms it); the slider that sets the link
// strength threshold; and the way back to the sentence that says what the picture is. Collapsed by
// default.
import { ChevronUp } from 'lucide-react';
import {
  useCallback,
  useEffect,
  useId,
  useMemo,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react';
import { LINK_KINDS, type LinkKind } from '../api/graph';
import { Button } from '../components/ui/button';
import { fileMark, type FileMark } from '../map/fileEncoding';
import { countLinksByKind } from '../map/linkEncoding';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { formatCount } from '../text/format';
import {
  FOLDER_MEANINGS,
  FOLDER_NAMES,
  LINK_KIND_MEANINGS,
  LINK_KIND_NAMES,
  MARK_MEANINGS,
  MARK_NAMES,
} from '../text/vocabulary';
import { CANVAS_SIZE, LINK_DASH_PX, SIZE, STROKE } from '../theme/tokens';
import { isKeyField, shortcutFor } from './shortcuts';

const MARK_ORDER: readonly FileMark[] = ['recent', 'suspect', 'unread', 'plain'];

/** The sample's box and the sizes of what is drawn in it, in its own pixels. */
const SAMPLE = {
  width: SIZE.swatchWidth,
  height: SIZE.swatchHeight,
  centreX: SIZE.swatchWidth / 2,
  centreY: SIZE.swatchHeight / 2,
  fileRadius: SIZE.swatchHeight / 4,
  ringRadius: SIZE.swatchHeight / 2 - STROKE.hairline,
  squareSide: SIZE.swatchHeight / 2,
  lineInset: STROKE.hairline,
} as const;

/** Counts the files showing each mark. */
function countMarks(scene: Scene, nowMs: number): Readonly<Record<FileMark, number>> {
  const counts: Record<FileMark, number> = { suspect: 0, recent: 0, unread: 0, plain: 0 };
  for (const file of scene.snapshot.files) counts[fileMark(file, nowMs, scene.hasReads)] += 1;
  return counts;
}

/** The sample's box, drawn in the sample's own pixels. */
function Sample({ children }: { children: ReactNode }): ReactElement {
  return (
    <svg
      className="legend-sample"
      viewBox={`0 0 ${SAMPLE.width} ${SAMPLE.height}`}
      aria-hidden="true"
    >
      {children}
    </svg>
  );
}

function LineSample({ kind }: { kind: LinkKind }): ReactElement {
  return (
    <Sample>
      <line
        x1={SAMPLE.lineInset}
        y1={SAMPLE.centreY}
        x2={SAMPLE.width - SAMPLE.lineInset}
        y2={SAMPLE.centreY}
        className="legend-link"
        strokeWidth={CANVAS_SIZE.linkFocusedWidthPx}
        strokeDasharray={LINK_DASH_PX[kind].join(' ')}
      />
    </Sample>
  );
}

/**
 * A file's sample with its shape: the suspect's second ring, the recent file's filled centre, the
 * never-read file's thinner, dimmed outline.
 */
function CircleSample({ mark }: { mark: FileMark }): ReactElement {
  const { centreX: cx, centreY: cy } = SAMPLE;
  return (
    <Sample>
      <circle cx={cx} cy={cy} r={SAMPLE.fileRadius} className="legend-file" data-mark={mark} />
      {mark === 'suspect' ? (
        <circle cx={cx} cy={cy} r={SAMPLE.ringRadius} className="legend-ring" />
      ) : null}
      {mark === 'recent' ? (
        <circle
          cx={cx}
          cy={cy}
          r={SAMPLE.fileRadius * CANVAS_SIZE.recentDotShare}
          className="legend-dot"
        />
      ) : null}
    </Sample>
  );
}

function FolderSample({ isCollapsed }: { isCollapsed: boolean }): ReactElement {
  const side = isCollapsed ? SAMPLE.squareSide / 2 : SAMPLE.squareSide;
  return (
    <Sample>
      {isCollapsed ? (
        <circle
          cx={SAMPLE.centreX}
          cy={SAMPLE.centreY}
          r={SAMPLE.ringRadius}
          className="legend-collapsed"
        />
      ) : null}
      <rect
        x={SAMPLE.centreX - side / 2}
        y={SAMPLE.centreY - side / 2}
        width={side}
        height={side}
        className="legend-hub"
      />
    </Sample>
  );
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
      <span className="legend-meaning">{props.meaning}</span>
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

/** The files' marks, each with how many files show it. */
function FileEntries({ scene, nowMs }: { scene: Scene; nowMs: number }): ReactElement {
  const markCounts = useMemo(() => countMarks(scene, nowMs), [scene, nowMs]);
  const marks = MARK_ORDER.filter((mark) => mark !== 'unread' || scene.hasReads);
  return (
    <ul className="legend-list">
      {marks.map((mark) => (
        <Entry
          key={mark}
          testId={`legend-${mark}`}
          sample={<CircleSample mark={mark} />}
          name={MARK_NAMES[mark]}
          meaning={MARK_MEANINGS[mark]}
          count={markCounts[mark]}
        />
      ))}
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

function LegendBody({ scene, nowMs }: { scene: Scene; nowMs: number }): ReactElement {
  const setIntroOpen = useMapStore((state) => state.setIntroOpen);
  return (
    <div className="legend-body" data-scroll-area="true">
      <h2 className="legend-heading">Files</h2>
      <FileEntries scene={scene} nowMs={nowMs} />
      <h2 className="legend-heading">Folders</h2>
      <FolderEntries scene={scene} />
      <h2 className="legend-heading">Links drawn</h2>
      <LinkEntries scene={scene} />
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
export function Legend({ scene, nowMs }: { scene: Scene; nowMs: number }): ReactElement {
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
        {isOpen ? <LegendBody scene={scene} nowMs={nowMs} /> : null}
      </div>
    </section>
  );
}
