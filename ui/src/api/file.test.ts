// Tests for one file's detail guard: an accepted detail, then one rejected input per rule of the
// contract, and the 404 for an id that is not a file.
import { afterEach, describe, expect, it, vi } from 'vitest';
import { fetchFileDetail, fileUrl, parseFileDetail } from './file';
import type { ApiErrorCode } from './guard';

/**
 * A detail with every kind of entry; `history_total` and `symbols_total` follow the lists given
 * unless an override sets them.
 */
function detail(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  const merged: Record<string, unknown> = {
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
    history_reader: 'on',
    symbols: [
      { name: 'Order', kind: 'class', start_line: 3, end_line: 40, is_nested: false, selected: 2 },
      { name: 'total', kind: 'method', start_line: 12, end_line: 18, is_nested: true, selected: 2 },
      {
        name: 'cancel',
        kind: 'method',
        start_line: 12,
        end_line: 30,
        is_nested: true,
        selected: 0,
      },
    ],
    selected: [
      { start_line: 10, end_line: 20, orders: 2 },
      { start_line: 10, end_line: 24, orders: 1 },
    ],
    told: [
      { kind: 'work_order', at_ms: 1762900000000, used: true },
      { kind: 'warning', at_ms: 1762800000000, used: null },
    ],
    editor_url: 'vscode://file/C:/repo/src/orders.rs:1',
    ...overrides,
  };
  const length = (key: string): number => {
    const list = merged[key];
    return Array.isArray(list) ? list.length : 0;
  };
  if (!('history_total' in overrides)) merged['history_total'] = length('history');
  if (!('symbols_total' in overrides)) merged['symbols_total'] = length('symbols');
  return merged;
}

describe('parseFileDetail accepts', () => {
  it('facts, history with an equal date, and told entries, keeping every field', () => {
    expect(parseFileDetail(detail(), 12)).toEqual({ ok: true, value: detail() });
  });

  it('a file with nothing stored', () => {
    expect(parseFileDetail(detail({ facts: [], history: [], told: [] }), 12).ok).toBe(true);
  });

  it('the latest dates and the longest subject and refs', () => {
    // The longest refs: a pull request "#" and 20 digits, then 3 issue references of 100
    // characters, each after a space: 21 + 3 x 101 = 324.
    const refs = `#${'9'.repeat(20)}${` ${'r'.repeat(100)}`.repeat(3)}`;
    expect(refs).toHaveLength(324);
    const latest = detail({
      history: [{ subject: 's'.repeat(200), refs, date_s: 8.64e12 }],
      told: [{ kind: 'warning', at_ms: 8.64e15, used: null }],
    });
    expect(parseFileDetail(latest, 12).ok).toBe(true);
  });

  it('an editor link turned off', () => {
    const off = detail({ editor_url: null });
    expect(parseFileDetail(off, 12)).toEqual({ ok: true, value: off });
  });

  it('an editor link of exactly 12,488 characters: a 200-character template and a 4,096-byte path written as %XX', () => {
    const url = `vscode://file/${'a'.repeat(12_488 - 'vscode://file/'.length)}`;
    expect(parseFileDetail(detail({ editor_url: url }), 12)).toEqual({
      ok: true,
      value: detail({ editor_url: url }),
    });
  });

  it('a longer editor link without its link, keeping the rest of the detail', () => {
    const url = `vscode://file/${'a'.repeat(12_489 - 'vscode://file/'.length)}`;
    expect(parseFileDetail(detail({ editor_url: url }), 12)).toEqual({
      ok: true,
      value: detail({ editor_url: null }),
    });
  });

  it('a fact of exactly 240 characters', () => {
    const facts = [{ id: 1, kind: 'note', status: 'candidate', text: 'x'.repeat(240) }];
    expect(parseFileDetail(detail({ facts }), 12).ok).toBe(true);
  });

  it('the 10 newest of 12 commits, and the first 500 of 640 symbols', () => {
    const history = Array.from({ length: 10 }, () => ({ subject: 's', refs: '', date_s: 1 }));
    const symbols = Array.from({ length: 500 }, (_, at) => ({
      name: `f${at}`,
      kind: 'function',
      start_line: at + 1,
      end_line: at + 1,
      is_nested: false,
      selected: 0,
    }));
    const capped = detail({ history, history_total: 12, symbols, symbols_total: 640 });
    const result = parseFileDetail(capped, 12);
    expect(
      result.ok ? [result.value.history_total, result.value.symbols_total] : result.error,
    ).toEqual([12, 640]);
  });

  it('each state of the history worker', () => {
    for (const reader of ['on', 'off', 'refused']) {
      const result = parseFileDetail(detail({ history: [], history_reader: reader }), 12);
      expect(result.ok ? result.value.history_reader : result.error).toBe(reader);
    }
  });
});

const fact = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({ facts: [{ id: 1, kind: 'note', status: 'active', text: 't', ...change }] });
const told = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({ told: [{ kind: 'work_order', at_ms: 1, used: false, ...change }] });
const history = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({ history: [{ subject: 's', refs: '', date_s: 1, ...change }] });
const symbol = (change: Record<string, unknown>): Record<string, unknown> =>
  detail({
    symbols: [
      { name: 'f', kind: 'function', start_line: 1, end_line: 9, is_nested: false, selected: 0 },
      {
        name: 'g',
        kind: 'function',
        start_line: 10,
        end_line: 12,
        is_nested: false,
        selected: 0,
        ...change,
      },
    ],
  });

const REJECTED: [string, unknown, ApiErrorCode][] = [
  ['a string at the top', 'file', 'bad_shape'],
  ['a missing editor link', detail({ editor_url: undefined }), 'bad_shape'],
  ['an editor link that is not text', detail({ editor_url: 7 }), 'bad_shape'],
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
  ['refs of 325 characters', history({ refs: '#'.repeat(325) }), 'bad_range'],
  ['a history total below the commits listed', detail({ history_total: 2 }), 'bad_range'],
  ['a history total of 12 with 3 listed', detail({ history_total: 12 }), 'bad_range'],
  ['a missing history total', detail({ history_total: undefined }), 'bad_shape'],
  ['an unknown history worker state', detail({ history_reader: 'paused' }), 'bad_kind'],
  ['a symbols total of 4 with 3 listed', detail({ symbols_total: 4 }), 'bad_range'],
  ['an unknown symbol kind', symbol({ kind: 'macro' }), 'bad_kind'],
  ['an empty symbol name', symbol({ name: '' }), 'bad_shape'],
  ['a symbol at line 0', symbol({ start_line: 0 }), 'bad_range'],
  ['a symbol ending before it starts', symbol({ start_line: 9, end_line: 8 }), 'bad_range'],
  ['a symbol selected by 21 work orders', symbol({ selected: 21 }), 'bad_range'],
  ['is_nested that is not a boolean', symbol({ is_nested: 'no' }), 'bad_shape'],
  [
    'symbols out of line order',
    detail({ symbols: [...(detail().symbols as unknown[])].reverse() }),
    'bad_order',
  ],
  [
    'a selected span repeated',
    detail({
      selected: [
        { start_line: 10, end_line: 20, orders: 1 },
        { start_line: 10, end_line: 20, orders: 1 },
      ],
    }),
    'bad_order',
  ],
  [
    'a selected span no work order selected',
    detail({ selected: [{ start_line: 1, end_line: 2, orders: 0 }] }),
    'bad_range',
  ],
  [
    'more than 500 symbols',
    detail({
      symbols: Array.from({ length: 501 }, (_, at) => ({
        name: 'f',
        kind: 'function',
        start_line: at + 1,
        end_line: at + 1,
        is_nested: false,
        selected: 0,
      })),
    }),
    'bad_range',
  ],
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
