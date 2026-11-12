//! How the experience hint is worded and held to its cap: a lead naming the earlier work and how
//! long before the task it ended, the files it changed, and one clause of an approach that did not
//! hold. Every word is the hint's own or a path, a runner's name or a count from the record.
//!
//! The line is held to [`MAX_EXPERIENCE_TOKENS`]: the clause is dropped first, then the files named
//! one by one down to one, the rest counted (`and 2 more`); a line that cannot fit with one file is
//! not given.

use std::fmt::Write as _;

use cortex_core::{RepoPath, estimate_tokens};

/// The most tokens the hint takes, as estimated: a sentence, not a report.
pub const MAX_EXPERIENCE_TOKENS: usize = 80;

/// The most files the hint names in any one list: more are counted, or, in a clause, the clause is
/// not said.
pub const MAX_HINT_FILES: usize = 3;

/// Milliseconds in an hour.
const HOUR_MS: i64 = 60 * 60 * 1_000;

/// Milliseconds in a day.
const DAY_MS: i64 = 24 * HOUR_MS;

/// The hint `lead` (`A similar task (2 weeks ago)`) gives for `files`, the files the work changed
/// as they are now, with `clause` after them when it fits: the first form within
/// [`MAX_EXPERIENCE_TOKENS`] of the clause with [`MAX_HINT_FILES`] files named, the same without the
/// clause, then fewer files named down to one. None when there is no file, or nothing fits.
pub(super) fn fitted(lead: &str, files: &[RepoPath], clause: Option<&str>) -> Option<String> {
    if files.is_empty() {
        return None;
    }
    let with_clause = clause.map(|clause| (Some(clause), MAX_HINT_FILES));
    let without = (1..=MAX_HINT_FILES).rev().map(|shown| (None, shown));
    with_clause
        .into_iter()
        .chain(without)
        .map(|(clause, shown)| line(lead, (files, shown), clause))
        // INVARIANT: budgeted (the hint is held to MAX_EXPERIENCE_TOKENS where it is written)
        .find(|line| estimate_tokens(line) <= MAX_EXPERIENCE_TOKENS)
}

/// The hint's line: `lead`, the first `shown` of `files` with the rest counted, and `clause`.
fn line(lead: &str, (files, shown): (&[RepoPath], usize), clause: Option<&str>) -> String {
    let named: Vec<RepoPath> = files.iter().take(shown).cloned().collect();
    let rest = files.len().saturating_sub(named.len());
    let mut text = format!("[cortex] {lead} changed ");
    if rest == 0 {
        text.push_str(&listed(&named));
    } else {
        let more = if rest == 1 { "file" } else { "files" };
        let _ = write!(
            text,
            "{} and {rest} more {more}",
            named
                .iter()
                .map(RepoPath::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    if let Some(clause) = clause {
        let _ = write!(text, "; {clause}");
    }
    text.push_str(".\n");
    text
}

/// `paths` as a sentence lists them: `a`, `a and b`, `a, b and c`.
pub(super) fn listed(paths: &[RepoPath]) -> String {
    let names: Vec<&str> = paths.iter().map(RepoPath::as_str).collect();
    match names.as_slice() {
        [] => String::new(),
        [only] => (*only).to_owned(),
        [first @ .., last] => format!("{} and {last}", first.join(", ")),
    }
}

/// How long ago `ago_ms` milliseconds is, in the largest unit that holds it whole: less than an
/// hour, hours under a day, days under two weeks, weeks under 60 days, 30-day months under a
/// year, then 365-day years.
///
/// ```text
/// 59 minutes -> less than an hour ago; 1 hour -> 1 hour ago; 13 days -> 13 days ago;
/// 14 days -> 2 weeks ago; 60 days -> 2 months ago; 365 days -> 1 year ago
/// ```
pub(super) fn age(ago_ms: i64) -> String {
    let (count, unit) = if ago_ms < HOUR_MS {
        return "less than an hour ago".to_owned();
    } else if ago_ms < DAY_MS {
        (ago_ms / HOUR_MS, "hour")
    } else if ago_ms < 14 * DAY_MS {
        (ago_ms / DAY_MS, "day")
    } else if ago_ms < 60 * DAY_MS {
        (ago_ms / (7 * DAY_MS), "week")
    } else if ago_ms < 365 * DAY_MS {
        (ago_ms / (30 * DAY_MS), "month")
    } else {
        (ago_ms / (365 * DAY_MS), "year")
    };
    let plural = if count == 1 { "" } else { "s" };
    format!("{count} {unit}{plural} ago")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths of `names`.
    fn paths(names: &[&str]) -> Result<Vec<RepoPath>, cortex_core::PathError> {
        names.iter().map(|name| RepoPath::new(name)).collect()
    }

    /// The units, each edge worked by hand from the bounds above.
    #[test]
    fn an_age_is_said_in_the_largest_whole_unit() {
        let cases = [
            (0, "less than an hour ago"),
            (HOUR_MS - 1, "less than an hour ago"),
            (HOUR_MS, "1 hour ago"),
            (DAY_MS - 1, "23 hours ago"),
            (DAY_MS, "1 day ago"),
            (14 * DAY_MS - 1, "13 days ago"),
            (14 * DAY_MS, "2 weeks ago"),
            (60 * DAY_MS - 1, "8 weeks ago"),
            (60 * DAY_MS, "2 months ago"),
            (365 * DAY_MS - 1, "12 months ago"),
            (365 * DAY_MS, "1 year ago"),
            (800 * DAY_MS, "2 years ago"),
        ];
        for (ago_ms, said) in cases {
            assert_eq!(age(ago_ms), said, "{ago_ms}");
        }
    }

    /// One, two and three files are listed as a sentence lists them; four with three named are
    /// three and one more file; five, three and two more files. With room, the clause follows.
    #[test]
    fn files_are_listed_then_counted() -> Result<(), cortex_core::PathError> {
        let lead = "A similar task (1 day ago)";
        let said = |names: &[&str], clause| -> Result<Option<String>, cortex_core::PathError> {
            Ok(fitted(lead, &paths(names)?, clause))
        };
        assert_eq!(
            said(&["a.py"], Some("it put a.py back"))?.as_deref(),
            Some("[cortex] A similar task (1 day ago) changed a.py; it put a.py back.\n")
        );
        assert_eq!(
            said(&["a.py", "b.py"], None)?.as_deref(),
            Some("[cortex] A similar task (1 day ago) changed a.py and b.py.\n")
        );
        assert_eq!(
            said(&["a.py", "b.py", "c.py", "d.py"], None)?.as_deref(),
            Some("[cortex] A similar task (1 day ago) changed a.py, b.py, c.py and 1 more file.\n")
        );
        assert_eq!(
            said(&["a.py", "b.py", "c.py", "d.py", "e.py"], None)?.as_deref(),
            Some(
                "[cortex] A similar task (1 day ago) changed a.py, b.py, c.py and 2 more files.\n"
            )
        );
        assert_eq!(said(&[], None)?, None);
        Ok(())
    }

    /// The cap: a clause that would pass it is dropped, then files named until the line fits; a
    /// path too long to fit with nothing else gives no line. The paths are 100 bytes each; counted
    /// by hand at four bytes a token, three files and the clause are 371 bytes (93 tokens), three
    /// files alone 353 (89), two named and one counted 264 (66), the first within 80.
    #[test]
    fn the_hint_is_held_to_its_cap() -> Result<(), cortex_core::PathError> {
        let long = |letter: char| format!("{}.py", letter.to_string().repeat(97));
        let names = [long('a'), long('b'), long('c')];
        let files = paths(&names.iter().map(String::as_str).collect::<Vec<_>>())?;
        let lead = "A similar task (1 day ago)";
        let fitted_line = fitted(lead, &files, Some("it put x.py back")).ok_or("no line");
        let expected = format!(
            "[cortex] {lead} changed {}, {} and 1 more file.\n",
            names[0], names[1]
        );
        assert_eq!(fitted_line.as_deref(), Ok(expected.as_str()));
        assert!(estimate_tokens(&expected) <= MAX_EXPERIENCE_TOKENS);
        let too_long = paths(&[&format!("{}.py", "z".repeat(400))])?;
        assert_eq!(fitted(lead, &too_long, None), None);
        Ok(())
    }
}
