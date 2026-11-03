//! The times transcripts write, read as UTC milliseconds.

/// Milliseconds in a day.
const DAY_MS: i64 = 86_400_000;

/// The UTC milliseconds `text` names when it is a UTC time of the form transcripts write,
/// `2026-11-02T23:04:17.241Z`: a date, `T`, hours, minutes, seconds, an optional fraction of a
/// second (milliseconds kept, finer digits dropped) and `Z`. `None` for anything else, including a
/// date before 1970 or a time zone other than UTC.
///
/// ```
/// assert_eq!(cortex_meter::utc_millis("1970-01-02T00:00:01.5Z"), Some(86_401_500));
/// assert_eq!(cortex_meter::utc_millis("2026-11-02T23:04:17+01:00"), None);
/// ```
#[must_use]
pub fn utc_millis(text: &str) -> Option<i64> {
    let (date, rest) = text.split_once('T')?;
    let clock = rest.strip_suffix('Z')?;
    let (year, month, day) = date_parts(date)?;
    let (whole, fraction) = clock.split_once('.').unwrap_or((clock, ""));
    let (hours, minutes, seconds) = clock_parts(whole)?;
    let millis = fraction_millis(fraction)?;
    let days = days_since_epoch(year, month, day)?;
    let time = ((hours * 60 + minutes) * 60 + seconds) * 1_000 + millis;
    days.checked_mul(DAY_MS)?.checked_add(time)
}

/// The year, month and day of `YYYY-MM-DD`, each in its range.
fn date_parts(date: &str) -> Option<(i64, i64, i64)> {
    let mut parts = date.split('-');
    let year = digits(parts.next()?, 4)?;
    let month = digits(parts.next()?, 2)?;
    let day = digits(parts.next()?, 2)?;
    let is_valid = parts.next().is_none()
        && (1..=12).contains(&month)
        && (1..=days_in_month(year, month)).contains(&day);
    is_valid.then_some((year, month, day))
}

/// The hours, minutes and seconds of `HH:MM:SS`, each in its range.
fn clock_parts(clock: &str) -> Option<(i64, i64, i64)> {
    let mut parts = clock.split(':');
    let hours = digits(parts.next()?, 2)?;
    let minutes = digits(parts.next()?, 2)?;
    let seconds = digits(parts.next()?, 2)?;
    let is_valid = parts.next().is_none() && hours < 24 && minutes < 60 && seconds < 60;
    is_valid.then_some((hours, minutes, seconds))
}

/// The milliseconds of a fraction of a second written after the point: none for no fraction.
fn fraction_millis(fraction: &str) -> Option<i64> {
    if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let kept: String = fraction.chars().chain("000".chars()).take(3).collect();
    kept.parse().ok()
}

/// The number `text` writes in exactly `count` ASCII digits.
fn digits(text: &str, count: usize) -> Option<i64> {
    let is_digits = text.len() == count && text.bytes().all(|byte| byte.is_ascii_digit());
    if is_digits { text.parse().ok() } else { None }
}

/// How many days `month` of `year` has.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 1970-01-01 to the date, by counting whole years of 365 or 366 days and then the
/// months of the year; `None` before 1970.
fn days_since_epoch(year: i64, month: i64, day: i64) -> Option<i64> {
    if year < 1970 {
        return None;
    }
    let leap_days = |up_to: i64| up_to / 4 - up_to / 100 + up_to / 400;
    let years = year - 1970;
    let before_year = years * 365 + leap_days(year - 1) - leap_days(1969);
    let before_month: i64 = (1..month).map(|earlier| days_in_month(year, earlier)).sum();
    Some(before_year + before_month + day - 1)
}

#[cfg(test)]
mod tests {
    use super::utc_millis;

    /// Times worked out by hand: the epoch; 2000-03-01, after a leap day in a year divisible by
    /// 400 (30 years of 365 days plus 7 leap days, 10,957, then 31 + 29 days); a time of
    /// day; a fraction's first three digits kept.
    #[test]
    fn times_read_as_utc_milliseconds() {
        assert_eq!(utc_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            utc_millis("2000-03-01T00:00:00Z"),
            Some(11_017 * 86_400_000)
        );
        assert_eq!(utc_millis("1970-01-01T01:02:03.004Z"), Some(3_723_004));
        assert_eq!(utc_millis("1970-01-01T00:00:00.98765Z"), Some(987));
        assert_eq!(utc_millis("1970-01-01T00:00:00.1Z"), Some(100));
    }

    /// Anything not a UTC time of that form names no time.
    #[test]
    fn other_texts_name_no_time() {
        for text in [
            "",
            "2026-11-02",
            "2026-11-02T23:04:17",
            "2026-11-02T23:04:17+00:00",
            "2026-02-29T00:00:00Z",
            "2026-13-01T00:00:00Z",
            "2026-11-02T24:00:00Z",
            "1969-12-31T23:59:59Z",
            "2026-11-02T23:04:17.x1Z",
            "+026-11-02T23:04:17Z",
        ] {
            assert_eq!(utc_millis(text), None, "{text}");
        }
    }
}
