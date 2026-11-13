// Tests for the warnings guard: an accepted list with every kind and state, then one refused input
// per rule, the warning's state among them.
import { describe, expect, it } from 'vitest';
import type { ApiErrorCode } from './guard';
import { parseWarnings } from './warnings';

function warning(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    at_ms: 1762900000000,
    kind: 'cochange',
    source: 4,
    target: 5,
    source_path: 'src/orders/book.rs',
    target_path: 'src/orders/mod.rs',
    strength: 0.62,
    state: 'open',
    ...overrides,
  };
}

function warnings(list: unknown[]): Record<string, unknown> {
  return { version: 1, warnings: list };
}

describe('parseWarnings', () => {
  it('reads every kind and state, each file by its id and path, null for what is not known', () => {
    const list = [
      warning(),
      warning({ kind: 'test', target: 7, strength: 1, state: 'followed' }),
      warning({
        kind: 'call_site',
        source: null,
        target: null,
        source_path: null,
        target_path: 'vendor/gen.rs',
        strength: 0.3,
        state: 'not_followed',
      }),
    ];
    expect(parseWarnings(warnings(list))).toEqual({
      ok: true,
      value: { version: 1, warnings: list },
    });
  });

  it.each<[string, unknown, ApiErrorCode]>([
    ['an unknown version', { version: 7, warnings: [] }, 'bad_version'],
    ['warnings that are not a list', { version: 1, warnings: null }, 'bad_shape'],
    ['an unknown state', warnings([warning({ state: 'dismissed' })]), 'bad_kind'],
    ['a state that is not text', warnings([warning({ state: true })]), 'bad_shape'],
    ['a missing state', warnings([warning({ state: undefined })]), 'bad_shape'],
    [
      'an origin note, which names no other file',
      warnings([warning({ kind: 'origin' })]),
      'bad_kind',
    ],
    ['a strength above 1', warnings([warning({ strength: 1.2 })]), 'bad_shape'],
    ['a negative strength', warnings([warning({ strength: -0.1 })]), 'bad_shape'],
    ['a fractional file id', warnings([warning({ target: 1.5 })]), 'bad_shape'],
    ['no target path field', warnings([warning({ target_path: undefined })]), 'bad_shape'],
    ['an absolute source path', warnings([warning({ source_path: '/etc/passwd' })]), 'bad_shape'],
    ['101 warnings', warnings(Array.from({ length: 101 }, () => warning())), 'bad_range'],
  ])('refuses %s', (_name, input, code) => {
    const result = parseWarnings(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});
