// Tests for the snapshot guard and the fetch call: one accepted snapshot, then one rejected
// input per rule of the contract, with the literal error code each must give.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApiErrorCode } from './guard';
import { fetchGraph, parseGraph } from './graph';

function file(id: number, path: string): Record<string, unknown> {
  return {
    id,
    path,
    lines: 10,
    interest: 0.5,
    tokens: 100,
    touched_ms: null,
    facts: 0,
    suspect: 0,
    reads: 3,
    commits: 2,
  };
}

function snapshot(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    version: 1,
    files: [file(0, 'a.rs'), file(1, 'b.rs'), file(2, 'c.rs')],
    links: [
      { from: 0, to: 1, kind: 'co_change', weight: 0.8, commits: 6 },
      { from: 0, to: 2, kind: 'co_change', weight: 0.2, commits: 0 },
      { from: 1, to: 2, kind: 'used_together', weight: 1, commits: 0 },
      { from: 0, to: 2, kind: 'work_order', weight: 0, commits: 0 },
    ],
    ...overrides,
  };
}

function withFile(index: number, change: Record<string, unknown>): Record<string, unknown> {
  const files = [file(0, 'a.rs'), file(1, 'b.rs'), file(2, 'c.rs')];
  files[index] = { ...files[index], ...change };
  return snapshot({ files });
}

describe('parseGraph accepts', () => {
  it('a well-formed snapshot, keeping every field', () => {
    const result = parseGraph(
      snapshot({ files: [{ ...file(0, 'a.rs'), touched_ms: 1762900000000 }], links: [] }),
    );
    expect(result).toEqual({
      ok: true,
      value: {
        version: 1,
        files: [{ ...file(0, 'a.rs'), touched_ms: 1762900000000 }],
        links: [],
      },
    });
  });

  it('three files and links of all three kinds', () => {
    const result = parseGraph(snapshot());
    expect(result.ok).toBe(true);
    if (result.ok)
      expect(result.value.links.map((link) => link.kind)).toEqual([
        'co_change',
        'co_change',
        'used_together',
        'work_order',
      ]);
  });

  it('nested paths and names that only start with a dot', () => {
    const files = [file(0, '.github/ci.yml'), file(1, 'src/a/b.rs'), file(2, 'src/a/c..d.rs')];
    expect(parseGraph(snapshot({ files, links: [] })).ok).toBe(true);
  });

  it('an empty snapshot', () => {
    expect(parseGraph(snapshot({ files: [], links: [] })).ok).toBe(true);
  });
});

const REJECTED: [string, unknown, ApiErrorCode][] = [
  ['null', null, 'bad_shape'],
  ['an array at the top', [], 'bad_shape'],
  ['an unknown version', snapshot({ version: 2 }), 'bad_version'],
  ['a missing version', { files: [], links: [] }, 'bad_version'],
  ['files that are not an array', snapshot({ files: {} }), 'bad_shape'],
  ['links that are not an array', snapshot({ links: null }), 'bad_shape'],
  ['a file that is not an object', snapshot({ files: ['a.rs'] }), 'bad_shape'],
  ['an id that is not the index', withFile(1, { id: 5 }), 'bad_order'],
  ['a missing path', withFile(0, { path: undefined }), 'bad_shape'],
  ['an empty path', withFile(0, { path: '' }), 'bad_shape'],
  ['a backslash in a path', withFile(0, { path: 'a\\b.rs' }), 'bad_shape'],
  ['an absolute path', withFile(0, { path: '/a.rs' }), 'bad_shape'],
  ['a path ending in /', withFile(0, { path: 'a/' }), 'bad_shape'],
  ['an empty segment', withFile(0, { path: 'a//b.rs' }), 'bad_shape'],
  ['a . segment', withFile(0, { path: './a.rs' }), 'bad_shape'],
  ['a .. segment', withFile(0, { path: 'a/../b.rs' }), 'bad_shape'],
  ['a control character', withFile(0, { path: 'a\nb.rs' }), 'bad_shape'],
  ['a zero-width space', withFile(0, { path: 'a\u200bb.rs' }), 'bad_shape'],
  ['a right-to-left mark', withFile(0, { path: 'a\u200fb.rs' }), 'bad_shape'],
  ['a right-to-left override', withFile(0, { path: 'src/\u202ecod.rs' }), 'bad_shape'],
  ['a first-strong isolate', withFile(0, { path: 'a\u2068b.rs' }), 'bad_shape'],
  ['a byte-order mark', withFile(0, { path: '\ufeffa.rs' }), 'bad_shape'],
  ['touched_ms past the latest date', withFile(0, { touched_ms: 8.64e15 + 1 }), 'bad_range'],
  ['lines as a string', withFile(0, { lines: '10' }), 'bad_shape'],
  ['negative tokens', withFile(0, { tokens: -1 }), 'bad_shape'],
  ['fractional facts', withFile(0, { facts: 1.5 }), 'bad_shape'],
  ['a missing suspect count', withFile(0, { suspect: undefined }), 'bad_shape'],
  ['interest above 1', withFile(0, { interest: 1.01 }), 'bad_shape'],
  ['interest below 0', withFile(0, { interest: -0.1 }), 'bad_shape'],
  ['touched_ms missing instead of null', withFile(0, { touched_ms: undefined }), 'bad_shape'],
  ['touched_ms negative', withFile(0, { touched_ms: -5 }), 'bad_shape'],
  ['a missing reads count', withFile(0, { reads: undefined }), 'bad_shape'],
  ['negative reads', withFile(0, { reads: -1 }), 'bad_shape'],
  ['commits as a string', withFile(2, { commits: '4' }), 'bad_shape'],
  ['fractional commits', withFile(1, { commits: 0.5 }), 'bad_shape'],
  [
    'a link with a missing commits count',
    snapshot({ links: [{ from: 0, to: 1, kind: 'co_change', weight: 1 }] }),
    'bad_shape',
  ],
  [
    'commits on a link that is not co_change',
    snapshot({ links: [{ from: 0, to: 1, kind: 'used_together', weight: 1, commits: 3 }] }),
    'bad_range',
  ],
  [
    'paths out of order',
    snapshot({ files: [file(0, 'b.rs'), file(1, 'a.rs')], links: [] }),
    'bad_order',
  ],
  [
    'a duplicated path',
    snapshot({ files: [file(0, 'a.rs'), file(1, 'a.rs')], links: [] }),
    'bad_order',
  ],
  [
    'a link to an id past the end',
    snapshot({ links: [{ from: 0, to: 3, kind: 'co_change', weight: 1, commits: 0 }] }),
    'bad_range',
  ],
  [
    'a link with from equal to to',
    snapshot({ links: [{ from: 1, to: 1, kind: 'co_change', weight: 1, commits: 0 }] }),
    'bad_range',
  ],
  [
    'a link with from above to',
    snapshot({ links: [{ from: 2, to: 1, kind: 'co_change', weight: 1, commits: 0 }] }),
    'bad_range',
  ],
  [
    'a link with a missing from',
    snapshot({ links: [{ to: 1, kind: 'co_change', weight: 1, commits: 0 }] }),
    'bad_shape',
  ],
  [
    'an unknown kind',
    snapshot({ links: [{ from: 0, to: 1, kind: 'imports', weight: 1, commits: 0 }] }),
    'bad_kind',
  ],
  [
    'a kind that is not a string',
    snapshot({ links: [{ from: 0, to: 1, kind: 3, weight: 1, commits: 0 }] }),
    'bad_shape',
  ],
  [
    'a weight above 1',
    snapshot({ links: [{ from: 0, to: 1, kind: 'co_change', weight: 2, commits: 0 }] }),
    'bad_shape',
  ],
  [
    'a missing weight',
    snapshot({ links: [{ from: 0, to: 1, kind: 'co_change', commits: 0 }] }),
    'bad_shape',
  ],
  [
    'links out of kind order',
    snapshot({
      links: [
        { from: 0, to: 1, kind: 'work_order', weight: 1, commits: 0 },
        { from: 0, to: 2, kind: 'co_change', weight: 1, commits: 0 },
      ],
    }),
    'bad_order',
  ],
  [
    'links out of from order within a kind',
    snapshot({
      links: [
        { from: 1, to: 2, kind: 'co_change', weight: 1, commits: 0 },
        { from: 0, to: 2, kind: 'co_change', weight: 1, commits: 0 },
      ],
    }),
    'bad_order',
  ],
  [
    'a repeated link',
    snapshot({
      links: [
        { from: 0, to: 1, kind: 'co_change', weight: 1, commits: 0 },
        { from: 0, to: 1, kind: 'co_change', weight: 1, commits: 0 },
      ],
    }),
    'bad_order',
  ],
];

describe('parseGraph rejects', () => {
  it.each(REJECTED)('%s', (_name, input, code) => {
    const result = parseGraph(input);
    expect(result.ok).toBe(false);
    if (!result.ok) expect(result.error.code).toBe(code);
  });

  it('with a message that names the field', () => {
    const result = parseGraph(
      snapshot({ links: [{ from: 0, to: 1, kind: 'imports', weight: 1, commits: 0 }] }),
    );
    expect(result).toEqual({
      ok: false,
      error: { code: 'bad_kind', message: 'links[0].kind: unknown kind "imports"' },
    });
  });
});

describe('fetchGraph', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  function stubFetch(response: Response | Error): void {
    vi.stubGlobal('fetch', () =>
      response instanceof Error ? Promise.reject(response) : Promise.resolve(response),
    );
  }

  it('returns the parsed snapshot for a 200 with valid JSON', async () => {
    stubFetch(new Response(JSON.stringify(snapshot())));
    const result = await fetchGraph(new AbortController().signal);
    expect(result.ok).toBe(true);
  });

  it('reports a status that is not 2xx', async () => {
    stubFetch(new Response('', { status: 500 }));
    const result = await fetchGraph(new AbortController().signal);
    expect(result).toEqual({
      ok: false,
      error: { code: 'http_status', message: 'the daemon answered 500' },
    });
  });

  it('reports a 404 as not found', async () => {
    stubFetch(new Response('', { status: 404 }));
    const result = await fetchGraph(new AbortController().signal);
    expect(result).toEqual({
      ok: false,
      error: { code: 'not_found', message: 'the daemon answered 404' },
    });
  });

  it('reports a body that is not JSON', async () => {
    stubFetch(new Response('<html>'));
    const result = await fetchGraph(new AbortController().signal);
    expect(result.ok ? 'accepted' : result.error.code).toBe('bad_json');
  });

  it('reports a network failure', async () => {
    stubFetch(new TypeError('fetch failed'));
    const result = await fetchGraph(new AbortController().signal);
    expect(result).toEqual({ ok: false, error: { code: 'network', message: 'fetch failed' } });
  });
});
