// Tests for the activity guard: an accepted list of events (a file by its id and its path, or by
// its path alone, or none), then one refused input per rule.
import { describe, expect, it } from 'vitest';
import { parseActivity } from './activity';
import type { ApiErrorCode } from './guard';

function event(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    at_ms: 1762900000000,
    kind: 'edit',
    session: '3f2a9c1e-0000',
    agent: 'claude',
    title: 'Claude Code',
    file: 4,
    path: 'src/orders/book.rs',
    ...overrides,
  };
}

function activity(events: unknown[]): Record<string, unknown> {
  return { version: 1, events };
}

describe('parseActivity', () => {
  it('reads each event, an agent as its name and title, and a session not stored as no agent', () => {
    const result = parseActivity(
      activity([
        event(),
        event({ kind: 'prompt', agent: null, title: null, file: null, path: null }),
        event({ file: null, path: 'vendor/gen.rs' }),
      ]),
    );
    expect(result).toEqual({
      ok: true,
      value: {
        version: 1,
        events: [
          {
            at_ms: 1762900000000,
            kind: 'edit',
            session: '3f2a9c1e-0000',
            agent: { name: 'claude', title: 'Claude Code' },
            file: 4,
            path: 'src/orders/book.rs',
          },
          {
            at_ms: 1762900000000,
            kind: 'prompt',
            session: '3f2a9c1e-0000',
            agent: null,
            file: null,
            path: null,
          },
          {
            at_ms: 1762900000000,
            kind: 'edit',
            session: '3f2a9c1e-0000',
            agent: { name: 'claude', title: 'Claude Code' },
            file: null,
            path: 'vendor/gen.rs',
          },
        ],
      },
    });
  });

  it('keeps the daemon order even when two hooks report their times out of step', () => {
    const result = parseActivity(activity([event({ at_ms: 5 }), event({ at_ms: 9 })]));
    expect(result.ok ? result.value.events.map((e) => e.at_ms) : result.error).toEqual([5, 9]);
  });

  it.each<[string, unknown, ApiErrorCode]>([
    ['an unknown version', { version: 2, events: [] }, 'bad_version'],
    ['events that are not a list', { version: 1, events: {} }, 'bad_shape'],
    ['an unknown kind', activity([event({ kind: 'browse' })]), 'bad_kind'],
    ['an agent without its title', activity([event({ title: null })]), 'bad_shape'],
    ['a title without its agent', activity([event({ agent: null })]), 'bad_shape'],
    ['an agent that is no snake name', activity([event({ agent: 'Claude Code' })]), 'bad_shape'],
    ['a session id with a line break', activity([event({ session: 'a\nb' })]), 'bad_shape'],
    ['an empty session id', activity([event({ session: '' })]), 'bad_shape'],
    ['a negative file id', activity([event({ file: -1 })]), 'bad_shape'],
    ['no path field at all', activity([event({ path: undefined })]), 'bad_shape'],
    ['a path that climbs out', activity([event({ path: '../secret.rs' })]), 'bad_shape'],
    ['a time past the latest date', activity([event({ at_ms: 8.64e15 + 1 })]), 'bad_range'],
    ['101 events', activity(Array.from({ length: 101 }, () => event())), 'bad_range'],
  ])('refuses %s', (_name, input, code) => {
    const result = parseActivity(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});
