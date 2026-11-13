// The receipts chart's data and geometry, as pure functions: which tasks it draws (those of the
// sessions whose usage Cortex reads, by start, the newest `CHART.maxTasks`), how many tasks there
// are in all (each session's total, not only the newest the daemon lists), where each task's mark
// sits in a plot of a given size, and which task the pointer is nearest. Two plots share the tasks
// and their horizontal places, one per measure, each on its own scale from 0, so no plot has two
// axes. The daemon lists each session's newest `MAX_TASKS_SHOWN` tasks, at least as many as the
// chart draws, so the newest it draws of those listed are the newest of all.
import type { SessionReceipt, TaskReceipt } from '../api/receipts';
import { CHART } from '../theme/tokens';

/** One task the chart draws, with the session it belongs to. */
export interface ChartTask {
  readonly task: TaskReceipt;
  readonly session: SessionReceipt;
}

/** The tasks the chart draws and what it leaves out. */
export interface ChartTasks {
  /** By start, then by number, oldest first. */
  readonly tasks: readonly ChartTask[];
  /** Tasks of sessions whose usage is read, drawn or not, listed or not. */
  readonly metered: number;
  /** Tasks of sessions whose usage Cortex does not read, which have no usage to draw. */
  readonly unmetered: number;
}

/** A measure the chart plots, one plot each. */
export interface ChartMeasure {
  readonly name: string;
  readonly valueOf: (task: TaskReceipt) => number;
}

/** The two measures, in the order the plots stack. */
export const CHART_MEASURES: readonly ChartMeasure[] = [
  { name: 'Input tokens per task', valueOf: (task) => task.usage.cumulative_input },
  { name: 'Turns per task', valueOf: (task) => task.usage.turns },
];

/** The tasks of `sessions` the chart draws: those whose usage is read, the newest `max` of them. */
export function chartTasks(
  sessions: readonly SessionReceipt[],
  max: number = CHART.maxTasks,
): ChartTasks {
  const listed: ChartTask[] = [];
  let metered = 0;
  let unmetered = 0;
  for (const session of sessions) {
    if (!session.is_metered) {
      unmetered += session.tasks_total;
      continue;
    }
    metered += session.tasks_total;
    for (const task of session.tasks) listed.push({ task, session });
  }
  // INVARIANT: deterministic (by start, then by the task's number, which is unique)
  listed.sort((a, b) => a.task.started_at_ms - b.task.started_at_ms || a.task.seq - b.task.seq);
  return { tasks: listed.slice(Math.max(0, listed.length - max)), metered, unmetered };
}

/** The horizontal place of mark `index` of `count` in a plot `widthPx` wide, inset at both ends. */
export function xOf(index: number, count: number, widthPx: number): number {
  const span = Math.max(0, widthPx - 2 * CHART.insetPx);
  if (count <= 1) return CHART.insetPx + span / 2;
  return CHART.insetPx + (index * span) / (count - 1);
}

/**
 * The vertical place of `value` on a scale from 0 at the bottom to `max` at the top of a plot
 * `heightPx` tall, inset at both ends; with a max of 0 every value sits on the baseline.
 */
export function yOf(value: number, max: number, heightPx: number): number {
  const span = Math.max(0, heightPx - 2 * CHART.insetPx);
  const share = max > 0 ? value / max : 0;
  return CHART.insetPx + (1 - share) * span;
}

/** The mark nearest the horizontal place `xPx` among `count` marks in a plot `widthPx` wide. */
export function nearestIndex(xPx: number, count: number, widthPx: number): number {
  if (count <= 1) return 0;
  const span = Math.max(1, widthPx - 2 * CHART.insetPx);
  const index = Math.round(((xPx - CHART.insetPx) * (count - 1)) / span);
  return Math.min(count - 1, Math.max(0, index));
}
