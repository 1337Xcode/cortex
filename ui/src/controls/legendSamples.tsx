// The small drawings beside each legend entry, in the shapes and colours the canvas draws: a link's
// dash pattern, a file's circle in a view's tone (thin when dim, with its ring or dot), a folder's
// square or disc, a fact badge on a rim, a live marker's diamond and its trail. Colours come from
// the stylesheet by role, so a sample always matches its tokens.
import type { ReactElement, ReactNode } from 'react';
import type { LinkKind } from '../api/graph';
import { CANVAS_SIZE, LINK_DASH_PX, SIZE, STROKE } from '../theme/tokens';
import type { ToneMark, ToneRole } from '../views/view';

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

/** A link kind's line, in its dash pattern. */
export function LineSample({ kind }: { kind: LinkKind }): ReactElement {
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
 * A file's circle in a tone's colour, thinner when the tone is dim, with the suspect's second ring
 * or the recent file's filled centre when the tone carries one.
 */
export function CircleSample(props: {
  role: ToneRole;
  isDim: boolean;
  /** Whether the circle is filled with a share of its tone, as a tinting view draws it. */
  isFilled?: boolean;
  mark: ToneMark;
}): ReactElement {
  const { centreX: cx, centreY: cy } = SAMPLE;
  return (
    <Sample>
      <circle
        cx={cx}
        cy={cy}
        r={SAMPLE.fileRadius}
        className="legend-file"
        data-role={props.role}
        data-dim={props.isDim}
        data-fill={props.isFilled === true}
      />
      {props.mark === 'ring' ? (
        <circle cx={cx} cy={cy} r={SAMPLE.ringRadius} className="legend-ring" />
      ) : null}
      {props.mark === 'dot' ? (
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

/** A folder's square, or a folder drawn whole: its disc around the square. */
export function FolderSample({ isCollapsed }: { isCollapsed: boolean }): ReactElement {
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

/** A file with a fact badge on its rim, up and to the right. */
export function BadgeSample(): ReactElement {
  const { centreX: cx, centreY: cy, fileRadius: r } = SAMPLE;
  return (
    <Sample>
      <circle cx={cx} cy={cy} r={r} className="legend-file" data-role="file" data-dim={false} />
      <circle
        cx={cx + r * Math.SQRT1_2}
        cy={cy - r * Math.SQRT1_2}
        r={CANVAS_SIZE.badgeRadiusPx}
        className="legend-badge"
      />
    </Sample>
  );
}

/** A live marker: a diamond around a file. */
export function DiamondSample(): ReactElement {
  const { centreX: cx, centreY: cy, ringRadius: r } = SAMPLE;
  return (
    <Sample>
      <circle
        cx={cx}
        cy={cy}
        r={SAMPLE.fileRadius}
        className="legend-file"
        data-role="file"
        data-dim={false}
      />
      <polygon
        points={`${cx},${cy - r} ${cx + r},${cy} ${cx},${cy + r} ${cx - r},${cy}`}
        className="legend-marker"
      />
    </Sample>
  );
}

/** A trail: a line back to an earlier file, with the dot on it. */
export function TrailSample(): ReactElement {
  const { centreY: cy, lineInset } = SAMPLE;
  const end = SAMPLE.width - lineInset - CANVAS_SIZE.trailStopPx;
  return (
    <Sample>
      <line x1={lineInset} y1={cy} x2={end} y2={cy} className="legend-trail" />
      <circle cx={end} cy={cy} r={CANVAS_SIZE.trailStopPx} className="legend-stop" />
    </Sample>
  );
}
