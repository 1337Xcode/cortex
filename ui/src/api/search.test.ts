// Tests for the search query rules and the answer guard: one row per rule of the contract.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApiErrorCode } from './guard';
import { fetchSearch, isValidQuery, parseSearch, searchUrl } from './search';

function answer(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    version: 1,
    query: 'order',
    results: [
      { kind: 'file', file: 12, name: 'orders.rs', line: null },
      { kind: 'symbol', file: 12, name: 'OrderBook', line: 40 },
    ],
    ...overrides,
  };
}

function withResult(result: Record<string, unknown>): Record<string, unknown> {
  return answer({ results: [result] });
}

describe('query rules', () => {
  it('accepts 1 to 100 characters, counting code points', () => {
    expect(isValidQuery('')).toBe(false);
    expect(isValidQuery('a')).toBe(true);
    expect(isValidQuery('x'.repeat(100))).toBe(true);
    expect(isValidQuery('x'.repeat(101))).toBe(false);
    // 100 astral characters are 200 UTF-16 units and still 100 characters.
    expect(isValidQuery('\u{1f600}'.repeat(100))).toBe(true);
  });

  it('encodes the text into a relative URL', () => {
    expect(searchUrl('a b&c/d')).toBe('api/search?q=a%20b%26c%2Fd');
  });
});

describe('parseSearch accepts', () => {
  it('file and symbol results for the asked query', () => {
    expect(parseSearch(answer(), 'order')).toEqual({ ok: true, value: answer() });
  });

  it('no results', () => {
    expect(parseSearch(answer({ results: [] }), 'order').ok).toBe(true);
  });
});

const TWENTY_ONE = Array.from({ length: 21 }, (_, i) => ({
  kind: 'file',
  file: i,
  name: `f${i}.rs`,
  line: null,
}));

const REJECTED: [string, unknown, ApiErrorCode][] = [
  ['an array at the top', [], 'bad_shape'],
  ['an unknown version', answer({ version: 0 }), 'bad_version'],
  ['an answer to another query', answer({ query: 'orde' }), 'bad_shape'],
  ['results that are not an array', answer({ results: {} }), 'bad_shape'],
  ['more than 20 results', answer({ results: TWENTY_ONE }), 'bad_range'],
  ['an unknown kind', withResult({ kind: 'module', file: 1, name: 'm', line: null }), 'bad_kind'],
  ['a missing file id', withResult({ kind: 'file', name: 'a.rs', line: null }), 'bad_shape'],
  ['an empty name', withResult({ kind: 'file', file: 1, name: '', line: null }), 'bad_shape'],
  [
    'a file result with a line',
    withResult({ kind: 'file', file: 1, name: 'a', line: 3 }),
    'bad_shape',
  ],
  [
    'a symbol without a line',
    withResult({ kind: 'symbol', file: 1, name: 'A', line: null }),
    'bad_shape',
  ],
  ['a symbol at line 0', withResult({ kind: 'symbol', file: 1, name: 'A', line: 0 }), 'bad_shape'],
  ['a missing line', withResult({ kind: 'file', file: 1, name: 'a' }), 'bad_shape'],
];

describe('parseSearch rejects', () => {
  it.each(REJECTED)('%s', (_name, input, code) => {
    const result = parseSearch(input, 'order');
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});

describe('fetchSearch', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('reports the 400 the daemon gives a bad query', async () => {
    vi.stubGlobal('fetch', () => Promise.resolve(new Response('', { status: 400 })));
    const result = await fetchSearch('order', new AbortController().signal);
    expect(result).toEqual({
      ok: false,
      error: { code: 'http_status', message: 'the daemon answered 400' },
    });
  });
});
