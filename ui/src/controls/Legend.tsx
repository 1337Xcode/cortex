// The collapsible legend: what each mark on the canvas means, with how many of each this repository
// has; the slider that sets the link strength threshold. Collapsed by default.
import { useId, useMemo, useState, type ReactElement } from 'react';
import { LINK_KINDS, type LinkKind } from '../api/graph';
import { fileMark, type FileMark } from '../map/fileEncoding';
import { countLinksByKind } from '../map/linkEncoding';
import type { Scene } from '../map/scene';
import { useMapStore } from '../state/store';
import { formatCount } from '../text/format';
import { CANVAS_SIZE, CHROME, LINK_DASH_PX } from '../theme/tokens';

/** What each link kind is called in the legend and the side panel. */
export const LINK_KIND_NAMES: Readonly<Record<LinkKind, string>> = {
  co_change: 'Changed together',
  used_together: 'Used together',
  work_order: 'Named in a work order',
};

const MARK_NAMES: Readonly<Record<FileMark, string>> = {
  suspect: 'Has a suspect fact',
  recent: 'Read or edited in the last 24 hours',
  unread: 'Never read by an agent',
  plain: 'Other files',
};

const MARK_COLORS: Readonly<Record<FileMark, string>> = {
  suspect: 'var(--color-suspect)',
  recent: 'var(--color-recent)',
  unread: 'var(--color-unread)',
  plain: 'var(--color-file)',
};

const MARK_ORDER: readonly FileMark[] = ['recent', 'suspect', 'unread', 'plain'];

/** Counts the files showing each mark. */
function countMarks(scene: Scene, nowMs: number): Readonly<Record<FileMark, number>> {
  const counts: Record<FileMark, number> = { suspect: 0, recent: 0, unread: 0, plain: 0 };
  for (const file of scene.snapshot.files) counts[fileMark(file, nowMs, scene.hasReads)] += 1;
  return counts;
}

function LineSample({ kind }: { kind: LinkKind }): ReactElement {
  return (
    <svg className="legend-sample" viewBox="0 0 28 8" aria-hidden="true">
      <line
        x1="1"
        y1="4"
        x2="27"
        y2="4"
        className="legend-link"
        strokeWidth={CANVAS_SIZE.linkFocusedWidthPx}
        strokeDasharray={LINK_DASH_PX[kind].join(' ')}
      />
    </svg>
  );
}

/**
 * A file's sample with its second channel, drawn schematically: the suspect ring around the
 * circle, the recent dot in its centre, the never-read file's thinner outline.
 */
function CircleSample({ mark }: { mark: FileMark }): ReactElement {
  const strokeWidth = mark === 'unread' ? CHROME.swatchUnreadStrokePx : CHROME.swatchFileStrokePx;
  return (
    <svg className="legend-sample legend-sample-tall" viewBox="0 0 28 14" aria-hidden="true">
      <circle
        cx="14"
        cy="7"
        r="4"
        className="legend-file"
        style={{ stroke: MARK_COLORS[mark], strokeWidth }}
      />
      {mark === 'suspect' ? (
        <circle
          cx="14"
          cy="7"
          r="6.2"
          className="legend-file legend-suspect-ring"
          style={{ strokeWidth: CHROME.swatchFileStrokePx }}
        />
      ) : null}
      {mark === 'recent' ? (
        <circle cx="14" cy="7" r={4 * CANVAS_SIZE.recentDotShare} className="legend-recent-dot" />
      ) : null}
    </svg>
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

function LegendBody({ scene, nowMs }: { scene: Scene; nowMs: number }): ReactElement {
  const minStrength = useMapStore((state) => state.minStrength);
  const linkCounts = useMemo(
    () => countLinksByKind(scene.snapshot.links, minStrength),
    [scene, minStrength],
  );
  const markCounts = useMemo(() => countMarks(scene, nowMs), [scene, nowMs]);
  const marks = MARK_ORDER.filter((mark) => mark !== 'unread' || scene.hasReads);
  return (
    <div className="legend-body">
      <h2 className="legend-heading">Links drawn</h2>
      <ul className="legend-list">
        {LINK_KINDS.map((kind) => (
          <li key={kind} data-testid={`legend-${kind}`}>
            <LineSample kind={kind} />
            <span className="legend-name">{LINK_KIND_NAMES[kind]}</span>
            <span className="legend-count">{formatCount(linkCounts[kind])}</span>
          </li>
        ))}
      </ul>
      <StrengthSlider />
      <h2 className="legend-heading">Files</h2>
      <ul className="legend-list">
        {marks.map((mark) => (
          <li key={mark} data-testid={`legend-${mark}`}>
            <CircleSample mark={mark} />
            <span className="legend-name">{MARK_NAMES[mark]}</span>
            <span className="legend-count">{formatCount(markCounts[mark])}</span>
          </li>
        ))}
        <li data-testid="legend-folders">
          <svg className="legend-sample" viewBox="0 0 28 8" aria-hidden="true">
            <rect x="11" y="1" width="6" height="6" className="legend-hub" />
          </svg>
          <span className="legend-name">Folders</span>
          <span className="legend-count">{formatCount(scene.layout.hubPath.length - 1)}</span>
        </li>
        <li data-testid="legend-collapsed">
          <svg className="legend-sample legend-sample-tall" viewBox="0 0 28 14" aria-hidden="true">
            <circle cx="14" cy="7" r="6" className="legend-collapsed" />
            <rect x="12" y="5" width="4" height="4" className="legend-hub" />
          </svg>
          <span className="legend-name">
            A whole folder when zoomed out, named with its file count
          </span>
          <span className="legend-count" />
        </li>
      </ul>
    </div>
  );
}

/** The legend, with a button that shows and hides it. */
export function Legend({ scene, nowMs }: { scene: Scene; nowMs: number }): ReactElement {
  const [isOpen, setIsOpen] = useState(false);
  const bodyId = useId();
  return (
    <section className="legend" aria-label="Legend">
      <button
        type="button"
        className="pill legend-toggle"
        aria-expanded={isOpen}
        aria-controls={bodyId}
        onClick={() => {
          setIsOpen(!isOpen);
        }}
      >
        <span className="chevron" aria-hidden="true" data-open={isOpen} />
        <span>Legend</span>
      </button>
      <div id={bodyId} hidden={!isOpen}>
        {isOpen ? <LegendBody scene={scene} nowMs={nowMs} /> : null}
      </div>
    </section>
  );
}
