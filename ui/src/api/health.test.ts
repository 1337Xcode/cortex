// Tests for the health guard: both index states accepted, with and without the daemon's report,
// then one refused input per rule: a refused answer, or a report the page cannot read refused on
// its own while the rest of the answer counts.
import { describe, expect, it } from 'vitest';
import type { ApiErrorCode } from './guard';
import { parseHealth } from './health';

/** The daemon's report as `api/health` sends it, every field set. */
function daemon(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    uptime_ms: 7_200_000,
    hooks: [{ hook: 'pre_tool', requests: 412, p50_us: 850, p95_us: 2400, p99_us: 9100 }],
    errors: [{ at_ms: 1762900000000, count: 3, message: 'the watcher lost an event' }],
    events_dropped: 0,
    failed_batches: 1,
    git: 'ready',
    git_detail: null,
    graph_nodes: 1200,
    graph_links: 5400,
    upkeep_at_ms: 1762890000000,
    history: 'off',
    ...overrides,
  };
}

function health(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return { version: '0.4.0', has_assets: true, index: 'ready', daemon: null, ...overrides };
}

/** The answer `health(overrides)` parses to, with no daemon report. */
function parsedHealth(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return { ...health(overrides), daemon: { status: 'absent' } };
}

describe('parseHealth', () => {
  it('accepts a daemon still reading and one that has finished', () => {
    expect(parseHealth(health({ index: 'reading' }))).toEqual({
      ok: true,
      value: parsedHealth({ index: 'reading' }),
    });
    expect(parseHealth(health())).toEqual({ ok: true, value: parsedHealth() });
  });

  it("reads the daemon's report, the graph's two counts as one size", () => {
    const result = parseHealth(health({ daemon: daemon() }));
    expect(result.ok ? result.value.daemon : result.error).toEqual({
      status: 'read',
      health: {
        uptime_ms: 7_200_000,
        hooks: [{ hook: 'pre_tool', requests: 412, p50_us: 850, p95_us: 2400, p99_us: 9100 }],
        errors: [{ at_ms: 1762900000000, count: 3, message: 'the watcher lost an event' }],
        events_dropped: 0,
        failed_batches: 1,
        git: 'ready',
        git_detail: null,
        graph: { nodes: 1200, links: 5400 },
        upkeep_at_ms: 1762890000000,
        history: 'off',
      },
    });
  });

  it('accepts a graph and upkeep not there yet, a hook it does not know, and a miner state named other', () => {
    const early = daemon({
      graph_nodes: null,
      graph_links: null,
      upkeep_at_ms: null,
      git: 'other',
      git_detail: 'read by the git program',
      hooks: [{ hook: 'pre_compact_v2', requests: 1, p50_us: 5, p95_us: 5, p99_us: 5 }],
    });
    const result = parseHealth(health({ daemon: early }));
    const report = result.ok ? result.value.daemon : null;
    expect(
      report?.status === 'read' ? [report.health.graph, report.health.upkeep_at_ms] : report,
    ).toEqual([null, null]);
  });

  it('accepts a miner detail of 1,000 characters, as the daemon cuts it', () => {
    const result = parseHealth(health({ daemon: daemon({ git_detail: 'd'.repeat(1000) }) }));
    expect(result.ok ? result.value.daemon.status : result.error).toBe('read');
  });

  it.each<[string, unknown, ApiErrorCode]>([
    ['a missing index', health({ index: undefined }), 'bad_shape'],
    ['an unknown index state', health({ index: 'indexing' }), 'bad_kind'],
    ['an empty version', health({ version: '' }), 'bad_shape'],
    ['a version of 65 characters', health({ version: '1'.repeat(65) }), 'bad_range'],
    ['has_assets that is not a boolean', health({ has_assets: 'yes' }), 'bad_shape'],
    ['a list', [], 'bad_shape'],
    ['no daemon field at all', { version: '0.4.0', has_assets: true, index: 'ready' }, 'bad_shape'],
  ])('refuses %s', (_name, input, code) => {
    const result = parseHealth(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });

  it.each<[string, unknown, ApiErrorCode]>([
    ['an unknown miner state', health({ daemon: daemon({ git: 'cloning' }) }), 'bad_kind'],
    [
      'an unknown history worker state',
      health({ daemon: daemon({ history: 'on_hold' }) }),
      'bad_kind',
    ],
    [
      'percentiles that fall',
      health({
        daemon: daemon({
          hooks: [{ hook: 'stop', requests: 3, p50_us: 900, p95_us: 800, p99_us: 1000 }],
        }),
      }),
      'bad_order',
    ],
    [
      'an empty hook name',
      health({
        daemon: daemon({ hooks: [{ hook: '', requests: 1, p50_us: 1, p95_us: 1, p99_us: 1 }] }),
      }),
      'bad_shape',
    ],
    [
      'eleven errors',
      health({
        daemon: daemon({
          errors: Array.from({ length: 11 }, () => ({ at_ms: 1, count: 1, message: 'm' })),
        }),
      }),
      'bad_range',
    ],
    [
      'an error that never happened',
      health({ daemon: daemon({ errors: [{ at_ms: 1, count: 0, message: 'm' }] }) }),
      'bad_range',
    ],
    [
      'an error message of 1,001 characters',
      health({ daemon: daemon({ errors: [{ at_ms: 1, count: 1, message: 'm'.repeat(1001) }] }) }),
      'bad_range',
    ],
    ['graph nodes without links', health({ daemon: daemon({ graph_links: null }) }), 'bad_shape'],
    ['a negative uptime', health({ daemon: daemon({ uptime_ms: -1 }) }), 'bad_shape'],
    [
      'a miner detail of 1,001 characters',
      health({ index: 'reading', daemon: daemon({ git_detail: 'd'.repeat(1001) }) }),
      'bad_range',
    ],
  ])("refuses the daemon's report with %s, and still reads the index", (_name, input, code) => {
    const result = parseHealth(input);
    const report = result.ok ? result.value.daemon : null;
    expect(report?.status === 'unreadable' ? report.error.code : report).toBe(code);
    expect(result.ok ? result.value.index : result.error).toBe((input as { index: string }).index);
  });
});
