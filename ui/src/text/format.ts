// Number, date and plural formatting for the page's text. Fixed to one locale and to UTC, so the
// same data always reads the same, whatever machine or time zone opens the map.

const COUNT_FORMAT = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });

/** A whole number with thousands separators: 52310 reads "52,310". */
export function formatCount(value: number): string {
  return COUNT_FORMAT.format(value);
}

/** "1 file", "2 files": the count and the noun in the right number. */
export function countOf(value: number, singular: string, plural = `${singular}s`): string {
  return `${formatCount(value)} ${value === 1 ? singular : plural}`;
}

/** A whole percentage of `part` in `whole`, "0%" when the whole is empty. */
export function formatPercent(part: number, whole: number): string {
  return `${whole === 0 ? 0 : Math.round((part / whole) * 100)}%`;
}

/** A calendar date in UTC as YYYY-MM-DD, from milliseconds since the epoch; "unknown date" for a time no date can hold. */
export function formatDate(epochMs: number): string {
  const date = new Date(epochMs);
  if (Number.isNaN(date.getTime())) return 'unknown date';
  const month = String(date.getUTCMonth() + 1).padStart(2, '0');
  const day = String(date.getUTCDate()).padStart(2, '0');
  return `${String(date.getUTCFullYear()).padStart(4, '0')}-${month}-${day}`;
}

/** The last segment of a `/`-separated path. */
export function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1);
}
