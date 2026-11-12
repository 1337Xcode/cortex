// Tokens and turns per task over time: two small plots stacked on the same tasks, one mark per
// task, each plot on its own scale from 0 (one axis each, never two on one plot). The task under
// the pointer, or the one picked with the arrow keys, is explained below the plots in one sentence
// with its numbers, the same sentence `cortex receipts` prints for it. The keyboard reaches the
// tasks through a native range input that the plots stand for; screen readers hear the sentence
// as its value.
import { useId, useMemo, useRef, useState, type PointerEvent, type ReactElement } from 'react';
import type { SessionReceipt } from '../api/receipts';
import { useScreenSize } from '../map/useScreenSize';
import { countOf, formatCount, formatDate } from '../text/format';
import { taskSentence } from '../text/receiptWords';
import { CHART } from '../theme/tokens';
import {
  CHART_MEASURES,
  chartTasks,
  nearestIndex,
  xOf,
  yOf,
  type ChartMeasure,
  type ChartTask,
} from './taskChart';

/** One plot: a measure's marks on its scale from 0, the explained task's line across it. */
function Plot({
  measure,
  tasks,
  index,
  widthPx,
}: {
  measure: ChartMeasure;
  tasks: readonly ChartTask[];
  index: number;
  widthPx: number;
}): ReactElement {
  const values = tasks.map(({ task }) => measure.valueOf(task));
  const max = Math.max(0, ...values);
  const height = CHART.plotHeight;
  const crossX = xOf(index, tasks.length, widthPx);
  return (
    <figure className="chart-plot">
      <figcaption className="chart-caption">
        <span>{measure.name}</span>
        <span className="chart-scale">0 to {formatCount(max)}</span>
      </figcaption>
      <svg className="chart-svg" width={widthPx} height={height} aria-hidden="true">
        <line
          className="chart-grid"
          x2={widthPx}
          y1={yOf(max, max, height)}
          y2={yOf(max, max, height)}
        />
        <line
          className="chart-baseline"
          x2={widthPx}
          y1={yOf(0, max, height)}
          y2={yOf(0, max, height)}
        />
        <line className="chart-cross" x1={crossX} x2={crossX} y2={height} />
        {values.map((value, at) => (
          <circle
            // The tasks are fixed for a render and drawn in order, so the place is the key.
            key={at}
            className="chart-dot"
            data-selected={at === index}
            cx={xOf(at, tasks.length, widthPx)}
            cy={yOf(value, max, height)}
            r={at === index ? CHART.selectedRadius : CHART.dotRadius}
            strokeWidth={CHART.dotRing * 2}
          />
        ))}
      </svg>
    </figure>
  );
}

/** What the chart draws, and what it leaves out, in one note. */
function chartNote(shown: number, metered: number, unmetered: number): string {
  const drawn =
    shown < metered
      ? `The newest ${formatCount(shown)} of ${countOf(metered, 'task')}, oldest on the left`
      : `${countOf(shown, 'task')}, oldest on the left`;
  const has = unmetered === 1 ? 'has' : 'have';
  const left =
    unmetered === 0 ? '' : `; ${formatCount(unmetered)} more ${has} no usage Cortex reads`;
  return `${drawn}${left}. Point at a mark or use the arrow keys to read one.`;
}

/** The chart of the tasks of `sessions`. */
export function ReceiptsChart({ sessions }: { sessions: readonly SessionReceipt[] }): ReactElement {
  const data = useMemo(() => chartTasks(sessions), [sessions]);
  const count = data.tasks.length;
  const [picked, setPicked] = useState(count - 1);
  const plotsRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const inputId = useId();
  const { width } = useScreenSize(plotsRef);
  const index = Math.min(Math.max(0, picked), count - 1);
  const shown = data.tasks[index];
  if (shown === undefined) {
    return (
      <p className="panel-empty" data-testid="receipts-chart-empty">
        No task of these sessions has usage Cortex reads, so there is nothing to draw.
      </p>
    );
  }
  const sentence = taskSentence(shown.task, true);
  const pointAt = (event: PointerEvent<HTMLDivElement>): void => {
    const box = event.currentTarget.getBoundingClientRect();
    setPicked(nearestIndex(event.clientX - box.left, count, box.width));
  };
  const [first, last] = [data.tasks[0], data.tasks.at(-1)];
  return (
    <div className="receipts-chart" data-testid="receipts-chart">
      <p className="section-note">{chartNote(count, data.metered, data.unmetered)}</p>
      <div
        className="chart-plots"
        ref={plotsRef}
        onPointerMove={pointAt}
        onPointerDown={(event) => {
          pointAt(event);
          inputRef.current?.focus();
        }}
      >
        {CHART_MEASURES.map((measure) => (
          <Plot
            key={measure.name}
            measure={measure}
            tasks={data.tasks}
            index={index}
            widthPx={width}
          />
        ))}
      </div>
      <p className="chart-axis" aria-hidden="true">
        <span>{first === undefined ? '' : formatDate(first.task.started_at_ms)}</span>
        <span>{last === undefined ? '' : formatDate(last.task.started_at_ms)}</span>
      </p>
      <label className="visually-hidden" htmlFor={inputId}>
        Task in the chart
      </label>
      <input
        id={inputId}
        ref={inputRef}
        className="chart-input visually-hidden"
        type="range"
        min={0}
        max={count - 1}
        step={1}
        value={index}
        aria-valuetext={sentence}
        onChange={(event) => {
          setPicked(Number(event.target.value));
        }}
      />
      <p className="chart-readout" data-testid="receipts-readout">
        {sentence}
      </p>
    </div>
  );
}
