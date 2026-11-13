// Tests for sending the person's word on a fact: the request itself (a POST in fetch's default
// mode with no body), each answer the daemon gives, answers the page cannot read, and no answer.
import { describe, expect, it } from 'vitest';
import {
  FACT_WORD_INIT,
  factWordUrl,
  mayHaveWritten,
  sendFactWord,
  type FactWordOutcome,
} from './factWord';

/** A stand-in for `fetch` that answers `status` with `body` and records what it was asked. */
function answering(
  status: number,
  body: unknown,
): {
  send: typeof fetch;
  asked: { url: string; init: RequestInit | undefined }[];
} {
  const asked: { url: string; init: RequestInit | undefined }[] = [];
  const send: typeof fetch = (input, init) => {
    asked.push({ url: input instanceof Request ? input.url : input.toString(), init });
    return Promise.resolve(
      new Response(typeof body === 'string' ? body : JSON.stringify(body), { status }),
    );
  };
  return { send, asked };
}

describe('the request', () => {
  it('is a POST with no body, to the relative address, in fetch default mode', async () => {
    const { send, asked } = answering(200, { version: 1, id: 3, status: 'active' });
    await sendFactWord(3, 'confirm', send);
    expect(asked).toHaveLength(1);
    expect(asked[0]?.url).toBe('api/facts/3/confirm');
    expect(asked[0]?.init?.method).toBe('POST');
    expect(asked[0]?.init?.body).toBeUndefined();
    // The daemon takes the word only with the page's Origin. Its same-origin referrer policy sends
    // that in every mode; the mode is still left to fetch's default, which no other part relies on.
    expect(asked[0]?.init?.mode).toBeUndefined();
    expect(FACT_WORD_INIT.mode).toBeUndefined();
    expect(factWordUrl(12, 'retire')).toBe('api/facts/12/retire');
  });
});

describe('the answer', () => {
  it.each<[string, number, unknown, 'confirm' | 'retire', FactWordOutcome]>([
    [
      'confirmed',
      200,
      { version: 1, id: 3, status: 'active' },
      'confirm',
      { kind: 'done', status: 'active' },
    ],
    [
      'retired',
      200,
      { version: 1, id: 3, status: 'retired' },
      'retire',
      { kind: 'done', status: 'retired' },
    ],
    [
      'refused by the writer',
      409,
      { version: 1, error: 'there is no fact 3' },
      'confirm',
      { kind: 'undone', why: 'refused', reason: 'there is no fact 3' },
    ],
    [
      'not written by the store',
      500,
      {
        version: 1,
        error:
          "the store failed to write it, which changed nothing: the daemon's status lists the error",
      },
      'confirm',
      {
        kind: 'undone',
        why: 'unwritten',
        reason:
          "the store failed to write it, which changed nothing: the daemon's status lists the error",
      },
    ],
    [
      'not taken in time',
      503,
      {
        version: 1,
        error: 'the daemon was busy and did not take the word, which changed nothing: try again',
      },
      'retire',
      {
        kind: 'undone',
        why: 'busy',
        reason: 'the daemon was busy and did not take the word, which changed nothing: try again',
      },
    ],
  ])('%s', async (_name, status, body, word, outcome) => {
    const { send } = answering(status, body);
    expect(await sendFactWord(3, word, send)).toEqual(outcome);
  });

  it.each<[string, number, unknown, string]>([
    ['a forbidden request', 403, '', 'it answered 403'],
    ['a path that names no word', 404, '', 'it answered 404'],
  ])(
    'says a client error, %s, was refused before anything was done',
    async (_name, status, body, reason) => {
      const { send } = answering(status, body);
      const outcome = await sendFactWord(3, 'confirm', send);
      expect(outcome).toEqual({ kind: 'undone', why: 'refused', reason });
      expect(mayHaveWritten(outcome)).toBe(false);
    },
  );

  it.each<[string, number, unknown, string]>([
    ['another fact than asked', 200, { version: 1, id: 4, status: 'active' }, 'bad_range'],
    [
      'a status the word does not leave',
      200,
      { version: 1, id: 3, status: 'retired' },
      'bad_range',
    ],
    ['an unknown status', 200, { version: 1, id: 3, status: 'suspect' }, 'bad_kind'],
    ['an unknown version', 200, { version: 2, id: 3, status: 'active' }, 'bad_version'],
    ['a refusal with no reason', 409, { version: 1 }, 'bad_shape'],
    ['a server error with an empty body', 500, '', 'bad_json'],
    ['a body that is not JSON', 200, 'ok', 'bad_json'],
    ['a status the daemon never gives', 502, '', 'http_status'],
  ])(
    'says an answer with %s could not be read, so the word may have been written',
    async (_name, status, body, code) => {
      const { send } = answering(status, body);
      const outcome = await sendFactWord(3, 'confirm', send);
      expect(outcome.kind === 'unread' ? outcome.error.code : outcome.kind).toBe(code);
      expect(mayHaveWritten(outcome)).toBe(true);
    },
  );

  it('says an answer whose body broke off could not be read', async () => {
    const broken = new ReadableStream<Uint8Array>({
      start(controller) {
        controller.error(new TypeError('network error'));
      },
    });
    const send: typeof fetch = () => Promise.resolve(new Response(broken, { status: 200 }));
    const outcome = await sendFactWord(3, 'confirm', send);
    expect(outcome.kind === 'unread' ? outcome.error.code : outcome.kind).toBe('bad_json');
  });

  it('says no answer came when the request failed, never throwing', async () => {
    const send: typeof fetch = () => Promise.reject(new TypeError('Failed to fetch'));
    const outcome = await sendFactWord(3, 'confirm', send);
    expect(outcome).toEqual({
      kind: 'unanswered',
      error: { code: 'network', message: 'Failed to fetch' },
    });
    expect(mayHaveWritten(outcome)).toBe(true);
  });

  it('reads the facts again after a word written or perhaps written, never after a refusal', () => {
    const undone = (why: 'refused' | 'unwritten' | 'busy') =>
      mayHaveWritten({ kind: 'undone', why, reason: 'r' });
    expect([
      mayHaveWritten({ kind: 'done', status: 'active' }),
      undone('refused'),
      undone('unwritten'),
      undone('busy'),
    ]).toEqual([true, false, false, true]);
  });
});
