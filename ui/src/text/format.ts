// Number, date and plural formatting for the page's text. Fixed to one locale and to UTC, so the
// same data always reads the same, whatever machine or time zone opens the map.

const COUNT_FORMAT = new Intl.NumberFormat('en-US', { maximumFractionDigits: 0 });

/** One decimal place with thousands separators: 1234.56 reads "1,234.6". */
const ONE_DECIMAL_FORMAT = new Intl.NumberFormat('en-US', {
  minimumFractionDigits: 1,
  maximumFractionDigits: 1,
});

/** A whole number with thousands separators: 52310 reads "52,310". */
export function formatCount(value: number): string {
  return COUNT_FORMAT.format(value);
}

/** "1 file", "2 files": the count and the noun in the right number. */
export function countOf(value: number, singular: string, plural = `${singular}s`): string {
  return `${formatCount(value)} ${value === 1 ? singular : plural}`;
}

/**
 * A whole percentage of `part` in `whole`, "0%" when the whole is empty. A share that is neither
 * none nor all never rounds to either: 2 of 1,595 (0.13%) is "under 1%", and 1,594 of 1,595
 * (99.94%) "over 99%".
 */
export function formatPercent(part: number, whole: number): string {
  if (whole === 0 || part <= 0) return '0%';
  if (part >= whole) return '100%';
  const percent = (part / whole) * 100;
  if (percent < 1) return 'under 1%';
  if (percent > 99) return 'over 99%';
  return `${Math.round(percent)}%`;
}

/**
 * A number from 0 to 1 to two places, never rounded to a false 0 or 1: "0.62", "0" and "1" for
 * the ends themselves, "under 0.01" and "over 0.99" for a value that rounds to an end without
 * being it.
 */
export function formatUnit(value: number): string {
  if (value <= 0) return '0';
  if (value >= 1) return '1';
  if (value < 0.005) return 'under 0.01';
  if (value >= 0.995) return 'over 0.99';
  return value.toFixed(2);
}

/** A time of day in UTC as HH:MM:SS, from milliseconds since the epoch. */
export function formatUtcTime(epochMs: number): string {
  const date = new Date(epochMs);
  if (Number.isNaN(date.getTime())) return 'unknown time';
  const parts = [date.getUTCHours(), date.getUTCMinutes(), date.getUTCSeconds()];
  return parts.map((part) => String(part).padStart(2, '0')).join(':');
}

/** A moment in UTC as "YYYY-MM-DD HH:MM:SS UTC", from milliseconds since the epoch. */
export function formatUtcMoment(epochMs: number): string {
  return `${formatDate(epochMs)} ${formatUtcTime(epochMs)} UTC`;
}

const SECOND_MS = 1000;
const MINUTE_MS = 60 * SECOND_MS;
const HOUR_MS = 60 * MINUTE_MS;
const DAY_MS = 24 * HOUR_MS;

/**
 * A length of time in its largest whole unit: "45 seconds", "2 minutes", "3 hours", "4 days".
 * Rounded down, so "2 minutes" is from 2 up to 3; below a second it is "under a second".
 */
export function formatSpan(ms: number): string {
  if (ms < SECOND_MS) return 'under a second';
  if (ms < MINUTE_MS) return countOf(Math.floor(ms / SECOND_MS), 'second');
  if (ms < HOUR_MS) return countOf(Math.floor(ms / MINUTE_MS), 'minute');
  if (ms < DAY_MS) return countOf(Math.floor(ms / HOUR_MS), 'hour');
  return countOf(Math.floor(ms / DAY_MS), 'day');
}

/** Microseconds as the health table writes them: "850 µs" below a millisecond, else "12.4 ms". */
export function formatMicros(us: number): string {
  if (us < 1000) return `${formatCount(us)} µs`;
  return `${ONE_DECIMAL_FORMAT.format(us / 1000)} ms`;
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
