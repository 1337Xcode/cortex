// Tests for the receipts chart's data and geometry, with places worked by hand: a plot 276 px wide
// and 72 px tall, its marks inset 12 px, so they spread over 252 px across and 48 px up.
import { describe, expect, it } from 'vitest';
import type { SessionReceipt, TaskReceipt } from '../api/receipts';
import { CHART } from '../theme/tokens';
import { chartTasks, nearestIndex, xOf, yOf } from './taskChart';

function task(seq: number, startedMs: number): TaskReceipt {
  return {
    task_id: `01JC0000000000000000000${String(seq).padStart(3, '0')}`,
    seq,
    started_at_ms: startedMs,
    ended_at_ms: null,
    outcome: null,
    held_out: null,
    tool_calls: 0,
    injected_tokens: 0,
    usage: {
      turns: 0,
      cumulative_input: 0,
      uncached_input: 0,
      cache_reads: 0,
      cache_writes: 0,
      output: 0,
      estimate: null,
    },
  };
}

function session(id: string, isMetered: boolean, tasks: TaskReceipt[]): SessionReceipt {
  const nothing = task(0, 0).usage;
  return {
    session_id: id,
    agent: isMetered ? 'claude' : 'codex',
    is_metered: isMetered,
    started_at_ms: 0,
    ended_at_ms: null,
    status: 'live',
    subagents: 0,
    tool_calls: 0,
    tasks,
    outside_tasks: nothing,
    outside_tool_calls: 0,
    total: nothing,
    compactions_left_out: 0,
    cache_breaks: [],
    primer_tokens: 0,
    injected_tokens: 0,
  };
}

describe('chartTasks', () => {
  const sessions = [
    session('a', true, [task(1, 100), task(4, 400)]),
    session('b', false, [task(2, 200), task(3, 300)]),
    // Two tasks starting at the same moment are ordered by their number.
    session('c', true, [task(6, 250), task(5, 250)]),
  ];

  it('draws the tasks whose usage is read, by start and then number, and counts the rest', () => {
    const drawn = chartTasks(sessions);
    expect(drawn.tasks.map(({ task: entry }) => entry.seq)).toEqual([1, 5, 6, 4]);
    expect([drawn.metered, drawn.unmetered]).toEqual([4, 2]);
  });

  it('keeps the newest when there are more than it draws', () => {
    expect(chartTasks(sessions, 2).tasks.map(({ task: entry }) => entry.seq)).toEqual([6, 4]);
    expect(CHART.maxTasks).toBeGreaterThan(0);
  });
});

describe('the plot geometry', () => {
  it('spreads marks evenly between the insets, and centres a lone one', () => {
    // Five marks over 252 px are 63 px apart, from 12 to 264.
    expect([0, 1, 2, 4].map((index) => xOf(index, 5, 276))).toEqual([12, 75, 138, 264]);
    expect(xOf(0, 1, 276)).toBe(138);
  });

  it('scales values from 0 at the bottom to the most at the top', () => {
    // 48 px of span: 0 sits at 60, the most at 12, half the most at 36.
    expect([0, 50, 100].map((value) => yOf(value, 100, 72))).toEqual([60, 36, 12]);
    expect(yOf(0, 0, 72)).toBe(60);
  });

  it('finds the mark nearest the pointer, clamped to the first and the last', () => {
    expect(nearestIndex(138, 5, 276)).toBe(2);
    expect(nearestIndex(100, 5, 276)).toBe(1);
    expect(nearestIndex(0, 5, 276)).toBe(0);
    expect(nearestIndex(400, 5, 276)).toBe(4);
    expect(nearestIndex(200, 1, 276)).toBe(0);
  });
});
