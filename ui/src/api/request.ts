// The one fetch path every endpoint uses: a GET relative to the page, so the access token in the
// page's URL is kept and only the daemon that served the page is asked, then the endpoint's
// parser. Network failures, statuses and bodies that are not JSON become typed errors here.
import type { ApiResult } from './guard';

/** Loads one endpoint; the store passes the signal so a newer request can cancel an older one. */
export type Fetcher<T> = (signal: AbortSignal) => Promise<ApiResult<T>>;

function describeFailure(cause: unknown): string {
  return cause instanceof Error ? cause.message : 'the request failed';
}

/**
 * Fetches `url` (relative to the page) and parses the JSON body with `parse`. A 404 is
 * `not_found`, any other status that is not 2xx is `http_status`.
 */
export async function fetchChecked<T>(
  url: string,
  signal: AbortSignal,
  parse: (value: unknown) => ApiResult<T>,
): Promise<ApiResult<T>> {
  let response: Response;
  try {
    // INVARIANT: local-only (a relative URL, so only the daemon that served the page is asked)
    response = await fetch(url, {
      signal,
      cache: 'no-store',
      headers: { accept: 'application/json' },
    });
  } catch (cause) {
    return { ok: false, error: { code: 'network', message: describeFailure(cause) } };
  }
  if (!response.ok) {
    const code = response.status === 404 ? 'not_found' : 'http_status';
    return { ok: false, error: { code, message: `the daemon answered ${response.status}` } };
  }
  let body: unknown;
  try {
    body = await response.json();
  } catch (cause) {
    return { ok: false, error: { code: 'bad_json', message: describeFailure(cause) } };
  }
  return parse(body);
}
