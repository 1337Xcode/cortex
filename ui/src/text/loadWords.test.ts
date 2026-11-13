import { describe, expect, it } from 'vitest';
import type { ApiError } from '../api/guard';
import { whyNotLoaded } from './loadWords';

/** The error as the API builds it, with the message each kind really carries. */
function error(code: ApiError['code'], message: string): ApiError {
  return { code, message };
}

describe('whyNotLoaded', () => {
  it('lets a status or a missing thing speak for itself', () => {
    expect(whyNotLoaded(error('http_status', 'the daemon answered 500'))).toBe(
      'the daemon answered 500',
    );
    expect(whyNotLoaded(error('not_found', 'the daemon answered 404'))).toBe(
      'the daemon answered 404',
    );
  });

  it("names a failure to reach Cortex, which the browser's own words do not", () => {
    expect(whyNotLoaded(error('network', 'Failed to fetch'))).toBe(
      'Cortex could not be reached (Failed to fetch)',
    );
  });

  it('names an answer the page cannot read, whichever way it was wrong', () => {
    const wrong: readonly ApiError['code'][] = [
      'bad_json',
      'bad_version',
      'bad_shape',
      'bad_range',
      'bad_order',
      'bad_kind',
    ];
    const said = wrong.map((code) => whyNotLoaded(error(code, 'facts[0].line is not a number')));
    expect(new Set(said)).toEqual(
      new Set(['Cortex sent an answer this page cannot read (facts[0].line is not a number)']),
    );
  });

  it('never shows the code, which means nothing to the person reading the page', () => {
    const codes: readonly ApiError['code'][] = [
      'network',
      'http_status',
      'not_found',
      'bad_json',
      'bad_version',
      'bad_shape',
      'bad_range',
      'bad_order',
      'bad_kind',
    ];
    for (const code of codes) {
      expect(whyNotLoaded(error(code, 'a message'))).not.toContain(code);
    }
  });
});
