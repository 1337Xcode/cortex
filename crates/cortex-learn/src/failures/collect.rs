//! What every parser shares: the [`Collector`] that gathers a parser's findings under the caps,
//! and the small scanners for numbers and error names that the formats have in common. Locations
//! are in `locate`, which paths are the project's in `paths`, the shortening and cleaning of the
//! output in `clean`.

use super::locate::split_location;
use super::paths::is_kept_path;
use super::{Counts, Failures, Frame, MAX_FAILED_TESTS, MAX_FRAMES, MAX_NAME_CHARS};

/// The order in which a runner prints one failure's frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TraceOrder {
    /// The frame that raised the error comes first (Node, Go, Java and Rust backtraces).
    InnermostFirst,
    /// The frame that raised the error comes last (Python tracebacks and pytest's styles).
    OutermostFirst,
}

/// The frames of the failure being read, and the order they were printed in.
struct Trace {
    /// The order the runner prints this failure's frames in.
    order: TraceOrder,
    /// The frames, as printed.
    frames: Vec<Frame>,
}

/// Gathers one parser's findings: counts add up, names and frames keep their first occurrence
/// and stop at their caps, and the first error kind stays.
///
/// Frames of one failure are gathered between [`Collector::begin_trace`] and
/// [`Collector::end_trace`] and added innermost first whatever order the runner printed them in.
/// A frame added outside a trace is a failure of its own with one frame, which needs no order
/// (a compiler diagnostic, a panic's location).
#[derive(Default)]
pub(super) struct Collector {
    /// The counts so far, added over every summary line given.
    counts: Option<Counts>,
    /// The failed tests' names.
    failed_tests: Vec<String>,
    /// The frames, each failure's innermost first.
    frames: Vec<Frame>,
    /// The failure whose frames are being read, if any.
    trace: Option<Trace>,
    /// The kind of the first error.
    error_kind: Option<String>,
}

impl Collector {
    /// Adds `more` to the counts, as a run of several test binaries prints one summary each. A
    /// passing count is there once any of the summaries has one.
    pub(super) fn add_counts(&mut self, more: Counts) {
        let total = self.counts.get_or_insert_with(Counts::default);
        total.failed = total.failed.saturating_add(more.failed);
        total.passed = match (total.passed, more.passed) {
            (None, None) => None,
            (sofar, now) => Some(sofar.unwrap_or(0).saturating_add(now.unwrap_or(0))),
        };
        total.errors = total.errors.saturating_add(more.errors);
        total.skipped = total.skipped.saturating_add(more.skipped);
    }

    /// Adds a failed test's name, cleaned by [`clean_name`], unless it is empty, already there or
    /// the list is full.
    pub(super) fn add_test(&mut self, name: &str) {
        if self.failed_tests.len() >= MAX_FAILED_TESTS {
            return;
        }
        let name = clean_name(name);
        if !name.is_empty() && !self.failed_tests.contains(&name) {
            self.failed_tests.push(name);
        }
    }

    /// Starts the frames of one failure, printed in `order`. A trace still open is ended first.
    pub(super) fn begin_trace(&mut self, order: TraceOrder) {
        self.end_trace();
        self.trace = Some(Trace {
            order,
            frames: Vec::new(),
        });
    }

    /// Ends the open trace and adds its frames, innermost first. Does nothing when none is open.
    pub(super) fn end_trace(&mut self) {
        let Some(mut trace) = self.trace.take() else {
            return;
        };
        if trace.order == TraceOrder::OutermostFirst {
            trace.frames.reverse();
        }
        for frame in trace.frames {
            self.push_frame(frame);
        }
    }

    /// Adds a frame at `line` of `path`, with `/` separators, unless the line is 0 or the path is
    /// one that the path rules drop. In a trace the frame waits for [`Collector::end_trace`].
    pub(super) fn add_frame(&mut self, path: &str, line: u32) {
        let path = path.trim().replace('\\', "/");
        if line == 0 || !is_kept_path(&path) {
            return;
        }
        let frame = Frame { path, line };
        match self.trace.as_mut() {
            Some(trace) => trace.frames.push(frame),
            None => self.push_frame(frame),
        }
    }

    /// Adds `frame` to the result unless it is there already or the list is full.
    fn push_frame(&mut self, frame: Frame) {
        if self.frames.len() < MAX_FRAMES && !self.frames.contains(&frame) {
            self.frames.push(frame);
        }
    }

    /// Adds the frame `text` names, written `path:line` or `path:line:column`, if it is one.
    pub(super) fn add_located(&mut self, text: &str) {
        if let Some((path, line)) = split_location(text) {
            self.add_frame(path, line);
        }
    }

    /// Sets the error kind, cleaned by [`clean_name`], unless one is already set.
    pub(super) fn set_error_kind(&mut self, kind: &str) {
        if self.error_kind.is_some() {
            return;
        }
        let kind = clean_name(kind);
        if !kind.is_empty() {
            self.error_kind = Some(kind);
        }
    }

    /// The findings, or `None` when they show no failure: the counts say nothing failed, or
    /// there is neither a failed test, a frame nor an error kind.
    pub(super) fn finish(mut self) -> Option<Failures> {
        self.end_trace();
        if let Some(counts) = self.counts
            && counts.failed == 0
            && counts.errors == 0
        {
            return None;
        }
        let shows_failure = self.counts.is_some()
            || !self.failed_tests.is_empty()
            || !self.frames.is_empty()
            || self.error_kind.is_some();
        shows_failure.then_some(Failures {
            counts: self.counts,
            failed_tests: self.failed_tests,
            frames: self.frames,
            error_kind: self.error_kind,
        })
    }
}

/// A name from the output made safe to keep and show: control characters and the invisible and
/// bidirectional-override characters become spaces, the ends are trimmed, and the rest is cut to
/// [`MAX_NAME_CHARS`]. Names come from the project's own output, which is untrusted text.
fn clean_name(text: &str) -> String {
    let spaced: String = text
        .chars()
        .map(|c| if is_unsafe_char(c) { ' ' } else { c })
        .collect();
    spaced.trim().chars().take(MAX_NAME_CHARS).collect()
}

/// Whether `c` is a control character or one that hides or reorders text when shown.
fn is_unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

/// The number at the start of `text` and what follows it, or `None` when it starts with no digit.
pub(super) fn take_digits(text: &str) -> Option<(u32, &str)> {
    let end = text
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(text.len());
    let number = text.get(..end)?.parse().ok()?;
    Some((number, text.get(end..)?))
}

/// The number at the start of `text`, after any spaces.
pub(super) fn leading_number(text: &str) -> Option<u32> {
    take_digits(text.trim_start()).map(|(number, _)| number)
}

/// The number just before the first `word` in `text` that has one: `1 failed` for `failed`.
pub(super) fn number_before(text: &str, word: &str) -> Option<u32> {
    text.match_indices(word)
        .find_map(|(at, _)| trailing_number(text.get(..at)?))
}

/// The number that ends `text`, ignoring spaces after it.
fn trailing_number(text: &str) -> Option<u32> {
    let text = text.trim_end();
    let start = text
        .char_indices()
        .rev()
        .find(|(_, c)| !c.is_ascii_digit())
        .map_or(0, |(at, c)| at + c.len_utf8());
    text.get(start..)?.parse().ok()
}

/// The number just after the first `label` in `text` that has one: `Failures: 1` for `Failures:`.
pub(super) fn number_after(text: &str, label: &str) -> Option<u32> {
    text.match_indices(label)
        .find_map(|(at, _)| leading_number(text.get(at + label.len()..)?))
}

/// A summary part written `N word` (`7 passed`) as its number and word.
pub(super) fn count_part(part: &str) -> Option<(u32, &str)> {
    let (number, word) = part.trim().split_once(' ')?;
    Some((number.parse().ok()?, word.trim()))
}

/// Sets the field of `counts` that `word` names (`failed`, `passed`, `skipped`, `error`,
/// `errors`) to `number`, and says whether it named one.
pub(super) fn apply_count(counts: &mut Counts, number: u32, word: &str) -> bool {
    match word {
        "failed" => counts.failed = number,
        "passed" => counts.passed = Some(number),
        "skipped" => counts.skipped = number,
        "error" | "errors" => counts.errors = number,
        _ => return false,
    }
    true
}

/// The name of the error `text` starts with, as Python and Java print it (`KeyError: 'a'`,
/// `java.lang.AssertionError: x`): an identifier, qualified or not, whose last part is
/// capitalized and is not a warning. `None` for prose, `OK` and `FAILED`.
pub(super) fn exception_name(text: &str) -> Option<&str> {
    let name = text.trim().split(':').next()?.trim();
    let last = name.rsplit('.').next()?;
    let starts_well = name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
    let is_identifier = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '$'));
    let is_named = last.starts_with(|c: char| c.is_ascii_uppercase())
        && last.chars().any(|c| c.is_ascii_lowercase())
        && !last.ends_with("Warning");
    (starts_well && is_identifier && is_named).then_some(name)
}

/// Whether `name` names an error type by its suffix, `TypeError` or `IOException`, which is how
/// JavaScript and Java name them and is what separates them from other capitalized words.
pub(super) fn is_error_type(name: &str) -> bool {
    name.ends_with("Error") || name.ends_with("Exception")
}
