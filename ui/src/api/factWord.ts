// The person's word on a fact, sent to the daemon as `cortex facts confirm` and `retire` send it:
// a POST to `api/facts/<number>/confirm` or `/retire` with no body. The daemon hands the word to
// its writer and answers once it is written: the fact's number and its status now (200), or why
// nothing was done in its own words (409 when the writer refused, 500 when the store failed to
// write it, 503 when it did not take the word in time, or may have taken it too late to say). The
// page never writes the store itself.
//
// The daemon takes a POST only with the map's own `Origin`. It serves the page with the
// `same-origin` referrer policy, under which the Fetch standard sends a same-origin POST with the
// page's origin whatever the request's mode (under `no-referrer`, any mode but `cors` would send
// `null`). The request goes in `fetch`'s default mode all the same: nothing here asks for another.
import {
  fail,
  readCappedString,
  readCount,
  readEnum,
  readRecord,
  readVersion,
  runParse,
  type ApiError,
  type ApiResult,
} from './guard';

/** What the person can say of a fact. */
export const FACT_WORDS = ['confirm', 'retire'] as const;

/** One of `FACT_WORDS`. */
export type FactWord = (typeof FACT_WORDS)[number];

/** The status each word leaves a fact in. */
export const STATUS_AFTER: Readonly<Record<FactWord, 'active' | 'retired'>> = {
  confirm: 'active',
  retire: 'retired',
};

/** The longest reason the page shows, in characters: the daemon's reasons are one short line. */
export const MAX_REASON_CHARS = 500;

/** The relative URL of `word` on fact `id`. */
export function factWordUrl(id: number, word: FactWord): string {
  // INVARIANT: local-only (relative to the page, so only the daemon that served it is asked)
  return `api/facts/${id}/${word}`;
}

/** How the word is sent: a POST with no body, in `fetch`'s default mode, never from a cache. */
export const FACT_WORD_INIT: RequestInit = {
  method: 'POST',
  cache: 'no-store',
  headers: { accept: 'application/json' },
};

/**
 * Why the daemon did not say the word was written, by its answer's status: the writer refused it
 * (409), the store failed to write it (500), or the writer did not take it, or answer, in time
 * (503, so it may yet have been written).
 */
export const UNDONE_BY_STATUS: ReadonlyMap<number, 'refused' | 'unwritten' | 'busy'> = new Map([
  [409, 'refused'],
  [500, 'unwritten'],
  [503, 'busy'],
]);

/** What came of the word. */
export type FactWordOutcome =
  /** Written: the fact's status now. */
  | { readonly kind: 'done'; readonly status: 'active' | 'retired' }
  /** The daemon did not say it was written, and said why in its words. */
  | {
      readonly kind: 'undone';
      readonly why: 'refused' | 'unwritten' | 'busy';
      readonly reason: string;
    }
  /**
   * No answer came: the request failed before any. A browser does not say whether that was before
   * the request went out (Cortex stopped) or after (its connection dropped while it worked).
   */
  | { readonly kind: 'unanswered'; readonly error: ApiError }
  /** An answer came that the page cannot read, so the word may or may not have been written. */
  | { readonly kind: 'unread'; readonly error: ApiError };

/** Whether `outcome` leaves the word possibly written, so the page should read the facts again. */
export function mayHaveWritten(outcome: FactWordOutcome): boolean {
  switch (outcome.kind) {
    case 'done':
    case 'unanswered':
    case 'unread':
      return true;
    case 'undone':
      return outcome.why === 'busy';
  }
}

/** Checks a 200's body: the fact asked about, in the status the word leaves it in. */
export function parseDone(
  value: unknown,
  id: number,
  word: FactWord,
): ApiResult<'active' | 'retired'> {
  return runParse((body) => {
    const record = readRecord(body, 'fact');
    readVersion(record, 'fact');
    const answered = readCount(record, 'id', 'fact');
    if (answered !== id) return fail('bad_range', `fact.id: asked for ${id}, got ${answered}`);
    const status = readEnum(record, 'status', ['active', 'retired'] as const, 'fact');
    if (status !== STATUS_AFTER[word]) {
      return fail('bad_range', `fact.status: ${word} leaves a fact ${STATUS_AFTER[word]}`);
    }
    return status;
  }, value);
}

/** Checks a refusal's body: the daemon's reason. */
export function parseRefusal(value: unknown): ApiResult<string> {
  return runParse((body) => {
    const record = readRecord(body, 'refusal');
    readVersion(record, 'refusal');
    return readCappedString(record, 'error', MAX_REASON_CHARS, 'refusal');
  }, value);
}

/** Reads `response`'s JSON body, or a `bad_json` error. */
async function bodyOf(response: Response): Promise<ApiResult<unknown>> {
  try {
    return { ok: true, value: await response.json() };
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : 'the body was not JSON';
    return { ok: false, error: { code: 'bad_json', message } };
  }
}

/** Sends `word` on fact `id` and says what came of it; never throws. */
export async function sendFactWord(
  id: number,
  word: FactWord,
  send: typeof fetch = fetch,
): Promise<FactWordOutcome> {
  let response: Response;
  try {
    response = await send(factWordUrl(id, word), FACT_WORD_INIT);
  } catch (cause) {
    const message = cause instanceof Error ? cause.message : 'the request failed';
    return { kind: 'unanswered', error: { code: 'network', message } };
  }
  const why = UNDONE_BY_STATUS.get(response.status);
  // Any other client error is refused before the daemon acts on the word (the Origin check, a
  // path that names no fact or word); any other status is an answer this page cannot read.
  if (why === undefined && isClientError(response.status)) {
    return { kind: 'undone', why: 'refused', reason: `it answered ${response.status}` };
  }
  if (why === undefined && response.status !== 200) {
    const message = `the daemon answered ${response.status}`;
    return { kind: 'unread', error: { code: 'http_status', message } };
  }
  const body = await bodyOf(response);
  if (!body.ok) return { kind: 'unread', error: body.error };
  if (why !== undefined) {
    const reason = parseRefusal(body.value);
    if (!reason.ok) return { kind: 'unread', error: reason.error };
    return { kind: 'undone', why, reason: reason.value };
  }
  const status = parseDone(body.value, id, word);
  return status.ok
    ? { kind: 'done', status: status.value }
    : { kind: 'unread', error: status.error };
}

/** Whether `status` is a client error (4xx). */
function isClientError(status: number): boolean {
  return status >= 400 && status < 500;
}
