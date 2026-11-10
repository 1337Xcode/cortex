//! Locations as the formats print them: `path:line`, `path:line:column`, and a line that starts
//! with one. Both scanners are linear in the length of the text.

use super::collect::take_digits;

/// The path and line of `text`, written `path:line` or `path:line:column` (a trailing colon is
/// allowed), or `None` when it is not written so. A Windows drive letter's colon is not taken for
/// a separator, and the last part of the path (after the last `/` or `\`) must hold a dot, so
/// prose such as `at the time: 12:30` is not a location.
pub(super) fn split_location(text: &str) -> Option<(&str, u32)> {
    let text = text.trim().trim_end_matches(':');
    let (rest, last) = text.rsplit_once(':')?;
    let last_number = last.parse::<u32>().ok()?;
    let (path, line) = match rest.rsplit_once(':') {
        Some((path, middle)) if middle.parse::<u32>().is_ok() => (path, middle.parse().ok()?),
        _ => (rest, last_number),
    };
    has_file_name(path).then_some((path, line))
}

/// Whether the last part of `path` looks like a file name: it holds a dot.
fn has_file_name(path: &str) -> bool {
    path.rsplit(['/', '\\'])
        .next()
        .is_some_and(|name| name.contains('.'))
}

/// A line that starts with a location, as most compilers and test runners print one.
pub(super) struct Located<'a> {
    /// The path as printed.
    pub(super) path: &'a str,
    /// The line.
    pub(super) line: u32,
    /// What follows the location's colon, with its leading space.
    pub(super) rest: &'a str,
}

/// The location a line starts with: `path:line:` or `path:line:column:` (the colon after the
/// number is required) followed by a space or the end of the line. The path holds no whitespace
/// and has a dot in it, so ordinary prose with a colon and a number in it is not a location.
///
/// The prefix is scanned once: a colon can end the path only after the line's first dot and
/// before its first whitespace, so the candidates are the colons between the two and no colon
/// makes the scan start over. Text with many colons and no whitespace costs one pass.
pub(super) fn locate_prefix(line: &str) -> Option<Located<'_>> {
    let first_dot = line.find('.')?;
    let first_space = line.find(char::is_whitespace).unwrap_or(line.len());
    let candidates = line.get(first_dot..first_space)?;
    candidates
        .match_indices(':')
        .find_map(|(at, _)| located_at(line, first_dot + at))
}

/// The location whose path ends at the colon at byte `colon` of `line`, if the line has one there.
fn located_at(line: &str, colon: usize) -> Option<Located<'_>> {
    let path = line.get(..colon)?;
    let (number, after_line) = take_digits(line.get(colon + 1..)?)?;
    let after_column = after_line
        .strip_prefix(':')
        .and_then(take_digits)
        .map_or(after_line, |(_, rest)| rest);
    let rest = after_column.strip_prefix(':')?;
    if !rest.is_empty() && !rest.starts_with(' ') {
        return None;
    }
    Some(Located {
        path,
        line: number,
        rest,
    })
}
