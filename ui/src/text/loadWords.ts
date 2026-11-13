// Why a load failed, in the words the person reading the page gets. The error's code names the
// kind of failure for the tests and the console; it is not shown, because a reader learns nothing
// from `http_status` that `the daemon answered 500` does not already say.
import type { ApiError } from '../api/guard';

/**
 * The reason half of a failure sentence: `${what} could not be loaded: ${whyNotLoaded(error)}.`
 *
 * A status or a missing thing speaks for itself, so its message stands alone. A network failure
 * and an answer the page cannot read are named first, because the browser's or the parser's own
 * words ("Failed to fetch", "facts[0].line is not a number") do not say which of the two happened.
 */
export function whyNotLoaded(error: ApiError): string {
  switch (error.code) {
    case 'http_status':
    case 'not_found':
      return error.message;
    case 'network':
      return `Cortex could not be reached (${error.message})`;
    default:
      return `Cortex sent an answer this page cannot read (${error.message})`;
  }
}
