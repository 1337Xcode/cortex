// Tests for the health guard: both index states accepted, then one refused input per rule.
import { describe, expect, it } from 'vitest';
import type { ApiErrorCode } from './guard';
import { parseHealth } from './health';

function health(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return { version: '0.4.0', has_assets: true, index: 'ready', ...overrides };
}

describe('parseHealth', () => {
  it('accepts a daemon still reading and one that has finished', () => {
    expect(parseHealth(health({ index: 'reading' }))).toEqual({
      ok: true,
      value: health({ index: 'reading' }),
    });
    expect(parseHealth(health())).toEqual({ ok: true, value: health() });
  });

  it.each<[string, unknown, ApiErrorCode]>([
    ['a missing index', health({ index: undefined }), 'bad_shape'],
    ['an unknown index state', health({ index: 'indexing' }), 'bad_kind'],
    ['an empty version', health({ version: '' }), 'bad_shape'],
    ['a version of 65 characters', health({ version: '1'.repeat(65) }), 'bad_range'],
    ['has_assets that is not a boolean', health({ has_assets: 'yes' }), 'bad_shape'],
    ['a list', [], 'bad_shape'],
  ])('refuses %s', (_name, input, code) => {
    const result = parseHealth(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});
