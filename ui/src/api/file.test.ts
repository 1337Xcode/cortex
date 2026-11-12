// Tests for one file's detail guard: an accepted detail, then one rejected input per rule of the
// contract, and the 404 for an id that is not a file.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchFileDetail, fileUrl, parseFileDetail } from './file';
import type { ApiErrorCode } from './guard';

function detail(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    version: 1,
    id: 12,
    path: 'src/orders.rs',
    facts: [
      { id: 7, kind: 'invariant', status: 'active', text: 'Totals are integer cents.' },
      { id: 9, kind: 'convention', status: 'suspect', text: 'Orders are frozen once paid.' },
    ],
    history: [
      { subject: 'Charge in integer cents', refs: '#482', date_s: 1762300000 },
      { subject: 'Split the order book', refs: '', date_s: 1762300000 },
      { subject: 'Add orders', refs: '#12', date_s: 1700000000 },
    ],
    told: [
      { kind: 'work_order', at_ms: 1762900000000, used: true },
      { kind: 'warning', at_ms: 1762800000000, used: null },
    ],
    editor_url: 'vscode://file/C:/repo/src/orders.rs:1',
    ...overrides,
  };
}

describe('parseFileDetail accepts', () => {
  it('facts, history with an equal date, and told entries, keeping every field', () => {
    expect(parseFileDetail(detail(), 12)).toEqual({ ok: true, value: detail() });
  });

  it('a file with nothing stored', () => {
    expect(parseFileDetail(detail({ facts: [], history: [], told: [] }), 12).ok).toBe(true);
  });

  it('the latest dates and the longest subject and refs', () => {
    const latest = detail({
      history: [{ subject: 's'.repeat(200), refs: 'r'.repeat(80), date_s: 8.64e12 }],
      told: [{ kind: 'warning', at_ms: 8.64e15, used: null }],
    });
    expect(parseFileDetail(latest, 12).ok).toBe(true);
  });

  it('an editor link turned off', () => {
    const off = detail({ editor_url: null });
    expect(parseFileDetail(off, 12)).toEqual({ ok: true, value: off });
  });

  it('an editor link of exactly 2,048 characters', () => {
    const url = `vscode://file/${'a'.repeat(2048 - 'vscode://file/'.length)}`;
    expect(parseFileDetail(detail({ editor_url: url }), 12).ok).toBe(true);
  });

  it('a fact of exactly 240 characters', () => {
    const facts = [{ id: 1, kind: 'note', status: 'candidate', text: 'x'.repeat(240) }];
    expect(parseFileDetail(detail({ facts }), 12).ok).toBe(true);
  });
});

const fact = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({ facts: [{ id: 1, kind: 'note', status: 'active', text: 't', ...change }] });
const told = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({ told: [{ kind: 'work_order', at_ms: 1, used: false, ...change }] });
const history = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({ history: [{ subject: 's', refs: '', date_s: 1, ...change }] });

const REJECTED: [string, unknown, ApiErrorCode][] = [
  ['a string at the top', 'file', 'bad_shape'],
  ['a missing editor link', detail({ editor_url: undefined }), 'bad_shape'],
  ['an editor link that is not text', detail({ editor_url: 7 }), 'bad_shape'],
  [
    'an editor link of 2,049 characters',
    detail({ editor_url: `vscode://file/${'a'.repeat(2049 - 'vscode://file/'.length)}` }),
    'bad_range',
  ],
  ['an unknown version', detail({ version: 3 }), 'bad_version'],
  ['another id than asked', detail({ id: 13 }), 'bad_range'],
  ['a path with a backslash', detail({ path: 'src\\orders.rs' }), 'bad_shape'],
  ['facts that are not an array', detail({ facts: null }), 'bad_shape'],
  [
    'facts out of id order',
    detail({ facts: [...(detail().facts as unknown[])].reverse() }),
    'bad_order',
  ],
  ['an unknown fact status', fact({ status: 'stale' }), 'bad_kind'],
  ['an empty fact kind', fact({ kind: '' }), 'bad_shape'],
  ['a fact text of 241 characters', fact({ text: 'x'.repeat(241) }), 'bad_range'],
  ['a fact text that is not a string', fact({ text: 5 }), 'bad_shape'],
  ['a negative fact id', fact({ id: -1 }), 'bad_shape'],
  [
    'history oldest first',
    detail({ history: [...(detail().history as unknown[])].reverse() }),
    'bad_order',
  ],
  [
    'more than 10 history entries',
    detail({ history: Array.from({ length: 11 }, () => ({ subject: 's', refs: '', date_s: 1 })) }),
    'bad_range',
  ],
  ['a history date that is not a count', history({ date_s: 1.5 }), 'bad_shape'],
  ['a missing refs string', history({ refs: undefined }), 'bad_shape'],
  ['told oldest first', detail({ told: [...(detail().told as unknown[])].reverse() }), 'bad_order'],
  [
    'more than 20 told entries',
    detail({ told: Array.from({ length: 21 }, () => ({ kind: 'warning', at_ms: 1, used: null })) }),
    'bad_range',
  ],
  ['an unknown told kind', told({ kind: 'primer' }), 'bad_kind'],
  ['used that is not a boolean or null', told({ used: 'yes' }), 'bad_shape'],
  ['a missing used field', told({ used: undefined }), 'bad_shape'],
  ['a told time past the latest date (8.64e15 ms)', told({ at_ms: 8.64e15 + 1 }), 'bad_range'],
  [
    'a history date past the latest date (8.64e12 s)',
    history({ date_s: 8.64e12 + 1 }),
    'bad_range',
  ],
  ['a subject of 201 characters', history({ subject: 'x'.repeat(201) }), 'bad_range'],
  ['refs of 81 characters', history({ refs: '#'.repeat(81) }), 'bad_range'],
];

describe('parseFileDetail rejects', () => {
  it.each(REJECTED)('%s', (_name, input, code) => {
    const result = parseFileDetail(input, 12);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});

describe('fetchFileDetail', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('asks the relative URL for the id', () => {
    expect(fileUrl(12)).toBe('api/file/12');
  });

  it('reports an id that is not a file as not found', async () => {
    vi.stubGlobal('fetch', () => Promise.resolve(new Response('', { status: 404 })));
    const result = await fetchFileDetail(99, new AbortController().signal);
    expect(result.ok ? 'accepted' : result.error.code).toBe('not_found');
  });
});
