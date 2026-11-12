// Tests for the summary guard: one accepted summary of each shape, then one rejected input per
// rule of the contract with the literal error code it must give.
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { ApiErrorCode } from './guard';
import { fetchSummary, parseSummary } from './summary';

function summary(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    version: 1,
    files: 1204,
    folders: 87,
    explored_files: 312,
    learned: true,
    strongest_pair: { from: 4, to: 9, commits: 31 },
    busiest_folder: { path: 'src/orders', tokens: 52310 },
    window_days: 7,
    history: { max_commits: 5000, max_age_days: 365, max_files_per_commit: 30 },
    ...overrides,
  };
}

describe('parseSummary accepts', () => {
  it('a learned repository, keeping every field', () => {
    expect(parseSummary(summary())).toEqual({ ok: true, value: summary() });
  });

  it('a cold repository with nothing to name', () => {
    const cold = summary({
      explored_files: 0,
      learned: false,
      strongest_pair: null,
      busiest_folder: null,
    });
    expect(parseSummary(cold)).toEqual({ ok: true, value: cold });
  });
});

const REJECTED: [string, unknown, ApiErrorCode][] = [
  ['null', null, 'bad_shape'],
  ['an unknown version', summary({ version: 2 }), 'bad_version'],
  ['a missing files count', summary({ files: undefined }), 'bad_shape'],
  ['a negative folders count', summary({ folders: -1 }), 'bad_shape'],
  ['explored files as a string', summary({ explored_files: '3' }), 'bad_shape'],
  ['more explored files than files', summary({ files: 3, explored_files: 4 }), 'bad_range'],
  ['learned that is not a boolean', summary({ learned: 1 }), 'bad_shape'],
  ['a pair that is not an object', summary({ strongest_pair: [4, 9] }), 'bad_shape'],
  ['a missing pair', summary({ strongest_pair: undefined }), 'bad_shape'],
  [
    'a pair with from above to',
    summary({ strongest_pair: { from: 9, to: 4, commits: 1 } }),
    'bad_range',
  ],
  [
    'a pair with fractional commits',
    summary({ strongest_pair: { from: 1, to: 2, commits: 1.5 } }),
    'bad_shape',
  ],
  [
    'a folder with an absolute path',
    summary({ busiest_folder: { path: '/src', tokens: 1 } }),
    'bad_shape',
  ],
  [
    'a folder with a .. segment',
    summary({ busiest_folder: { path: 'a/../b', tokens: 1 } }),
    'bad_shape',
  ],
  [
    'a folder with negative tokens',
    summary({ busiest_folder: { path: 'src', tokens: -2 } }),
    'bad_shape',
  ],
  ['a missing window', summary({ window_days: undefined }), 'bad_shape'],
  ['missing history limits', summary({ history: undefined }), 'bad_shape'],
  ['history limits that are not an object', summary({ history: [5000, 365, 30] }), 'bad_shape'],
  [
    'history limits without the most commits',
    summary({ history: { max_age_days: 365, max_files_per_commit: 30 } }),
    'bad_shape',
  ],
  [
    'a negative age in the history limits',
    summary({ history: { max_commits: 5000, max_age_days: -1, max_files_per_commit: 30 } }),
    'bad_shape',
  ],
  [
    'a fractional files per commit in the history limits',
    summary({ history: { max_commits: 5000, max_age_days: 365, max_files_per_commit: 2.5 } }),
    'bad_shape',
  ],
  [
    'not learned while files were read',
    summary({ learned: false, strongest_pair: null }),
    'bad_range',
  ],
  [
    'not learned while a pair is stored',
    summary({ learned: false, explored_files: 0 }),
    'bad_range',
  ],
];

describe('parseSummary rejects', () => {
  it.each(REJECTED)('%s', (_name, input, code) => {
    const result = parseSummary(input);
    expect(result.ok ? 'accepted' : result.error.code).toBe(code);
  });
});

describe('fetchSummary', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('asks the relative URL api/summary and parses the body', async () => {
    const asked: string[] = [];
    vi.stubGlobal('fetch', (url: string) => {
      asked.push(url);
      return Promise.resolve(new Response(JSON.stringify(summary())));
    });
    const result = await fetchSummary(new AbortController().signal);
    expect(asked).toEqual(['api/summary']);
    expect(result.ok).toBe(true);
  });
});
