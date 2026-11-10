//! jest and vitest: the `Tests:` summary of jest (`1 failed, 7 passed, 8 total`) and vitest's
//! (`1 failed | 7 passed (8)`), the `FAIL` lines, the `●` headers of jest's failure blocks, the
//! `✕` and `×` lines of verbose runs, vitest's `❯` frames and the stack frames of both.
//!
//! The formats name a failed test more than once and not alike, so only one kind of name is
//! kept: the failure headers when there are any, else the marked lines, else the failing files.
//!
//! A suite that failed to run (jest's `● Test suite failed to run`, vitest's `FAIL  x.test.ts
//! [ x.test.ts ]` of an import that cannot resolve) ran no tests. Such a suite is one error and
//! is named by its file, the bracket suffix removed. The counts of the `Tests` line are kept when
//! tests failed, with the suites that ran no tests added as errors; when no test failed, every
//! failed suite on the `Test Suites` or `Test Files` line is an error. The frames of each
//! failure are one trace, which Node prints innermost first.

use super::collect::{
    Collector, TraceOrder, apply_count, count_part, exception_name, is_error_type,
};
use super::locate::split_location;
use super::node_stack;
use super::{Counts, Failures};

/// What a run of lines has told so far.
#[derive(Default)]
struct Scan<'a> {
    /// Frames, the error kind, and the counts to add.
    collector: Collector,
    /// Names from `●` headers and from `FAIL file > suite > test` lines.
    headers: Vec<&'a str>,
    /// Names from `✕` and `×` lines.
    marks: Vec<&'a str>,
    /// Files from `FAIL file` lines.
    suites: Vec<&'a str>,
    /// The suites that failed to run, each once.
    unrun: Vec<&'a str>,
    /// The file of the latest `FAIL file` line.
    current_suite: Option<&'a str>,
    /// The counts of the `Tests` line.
    counts: Option<Counts>,
    /// The counts of the `Test Suites` or `Test Files` line.
    suite_counts: Option<Counts>,
    /// Whether a failure's header has been seen, from which error lines name the error.
    in_failure: bool,
}

/// Reads jest's or vitest's output.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut scan = Scan::default();
    for line in output.lines() {
        scan.read(line);
    }
    scan.finish()
}

impl<'a> Scan<'a> {
    /// Reads one line.
    fn read(&mut self, line: &'a str) {
        let trimmed = line.trim();
        if let Some(counts) = read_counts(trimmed, &["Tests:", "Tests "]) {
            self.counts = Some(counts);
        } else if let Some(counts) = read_counts(trimmed, &["Test Suites:", "Test Files "]) {
            self.suite_counts = Some(counts);
        } else if trimmed.starts_with("● Test suite failed to run") {
            self.mark_unrun(self.current_suite.unwrap_or_default());
            self.begin_failure();
        } else if let Some(name) = bullet_header(trimmed) {
            self.headers.push(name);
            self.begin_failure();
        } else if let Some(rest) = fail_rest(trimmed) {
            self.read_fail(rest);
        } else if let Some(name) = marked_name(trimmed) {
            self.marks.push(name);
        } else if let Some(location) = vitest_location(trimmed) {
            self.collector.add_located(location);
        } else if self.in_failure
            && let Some((path, number)) = node_stack::frame_in(line)
        {
            self.collector.add_frame(path, number);
        } else if self.in_failure
            && let Some(kind) = error_type(trimmed)
        {
            self.collector.set_error_kind(kind);
        }
    }

    /// Reads what follows `FAIL`: a test (`file > suite > test`), a failing file, or vitest's
    /// `file [ file ]` of a suite that failed to run.
    fn read_fail(&mut self, rest: &'a str) {
        if rest.contains(" > ") {
            self.headers.push(rest);
        } else if let Some(name) = import_failure_name(rest) {
            self.suites.push(name);
            self.current_suite = Some(name);
            self.mark_unrun(name);
        } else {
            self.suites.push(rest);
            self.current_suite = Some(rest);
        }
        self.begin_failure();
    }

    /// Starts a failure: its frames are one trace, and error lines now name the error.
    fn begin_failure(&mut self) {
        self.collector.begin_trace(TraceOrder::InnermostFirst);
        self.in_failure = true;
    }

    /// Records that the suite `name` failed to run, once.
    fn mark_unrun(&mut self, name: &'a str) {
        if !self.unrun.contains(&name) {
            self.unrun.push(name);
        }
    }

    /// The findings; see the module docs for how the counts and the suites that failed to run
    /// are put together.
    fn finish(mut self) -> Option<Failures> {
        let unrun = u32::try_from(self.unrun.len()).unwrap_or(u32::MAX);
        let suites_failed = self.suite_counts.map_or(0, |found| found.failed);
        let has_failed_tests = self
            .counts
            .is_some_and(|found| found.failed > 0 || found.errors > 0);
        let suite_errors = if has_failed_tests {
            unrun
        } else {
            unrun.max(suites_failed)
        };
        let mut counts = self.counts;
        if suite_errors > 0 {
            let mut found = counts.unwrap_or_default();
            found.errors = found.errors.saturating_add(suite_errors);
            counts = Some(found);
        }
        if let Some(found) = counts {
            self.collector.add_counts(found);
        }
        let names = [&self.headers, &self.marks, &self.suites]
            .into_iter()
            .find(|names| !names.is_empty());
        for name in names.into_iter().flatten().chain(&self.unrun) {
            self.collector.add_test(name);
        }
        self.collector.finish()
    }
}

/// The file of vitest's `src/x.test.ts [ src/x.test.ts ]`, a suite that failed to import.
fn import_failure_name(rest: &str) -> Option<&str> {
    let (name, _) = rest.strip_suffix(" ]")?.split_once(" [ ")?;
    let name = name.trim();
    (!name.is_empty()).then_some(name)
}

/// The counts of a summary line that starts with one of `prefixes`, its parts written `N word`
/// and separated by commas or bars; `None` when no part names a count.
fn read_counts(line: &str, prefixes: &[&str]) -> Option<Counts> {
    let body = prefixes
        .iter()
        .find_map(|prefix| line.strip_prefix(*prefix))?;
    // Both tools print only the counts that are not zero, so a missing `passed` is 0.
    let mut counts = Counts {
        passed: Some(0),
        ..Counts::default()
    };
    let mut found = false;
    for part in body.split([',', '|']) {
        let part = part.split(" (").next().unwrap_or_default();
        if let Some((number, word)) = count_part(part) {
            // A line of `0 total` still printed the counts: every one is 0.
            found |= apply_count(&mut counts, number, word) || word == "total";
        }
    }
    found.then_some(counts)
}

/// The name in a `● suite › test` header. The headers of console output, of warnings and of a
/// suite that failed to run name no test.
fn bullet_header(line: &str) -> Option<&str> {
    let name = line.strip_prefix('●')?.trim();
    let names_no_test = name.is_empty()
        || name.ends_with(':')
        || name == "Console"
        || name.starts_with("Test suite failed to run");
    (!names_no_test).then_some(name)
}

/// What follows `FAIL` on a line that starts with it, without the time jest may print after it.
fn fail_rest(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("FAIL")?;
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let rest = strip_timing(rest.trim());
    (!rest.is_empty()).then_some(rest)
}

/// The name on a `✕ name (3 ms)` or `× name 2ms` line.
fn marked_name(line: &str) -> Option<&str> {
    let name = strip_timing(line.strip_prefix(['✕', '×', '✗'])?.trim());
    (!name.is_empty()).then_some(name)
}

/// The location in a vitest `❯ path:line:column` line, which may name the function before it.
fn vitest_location(line: &str) -> Option<&str> {
    let token = line.strip_prefix('❯')?.split_whitespace().last()?;
    split_location(token).map(|_| token)
}

/// The error type at the start of a line: `TypeError` in `TypeError: x is not a function`.
fn error_type(line: &str) -> Option<&str> {
    let name = line.split([':', ' ']).next()?;
    let rest = line.get(name.len()..)?;
    if !rest.starts_with(':') || !is_error_type(name) {
        return None;
    }
    exception_name(name)
}

/// `name` without the duration that ends it, `(3 ms)` or `2ms` or `(5.1 s)`.
fn strip_timing(name: &str) -> &str {
    let name = name.trim_end();
    if let Some((head, tail)) = name.rsplit_once(" (")
        && is_duration(&tail.trim_end_matches(')').replace(' ', ""))
    {
        return head.trim_end();
    }
    if let Some((head, tail)) = name.rsplit_once(' ')
        && is_duration(tail)
    {
        return head.trim_end();
    }
    name
}

/// Whether `text` is a duration: digits, perhaps a point, and `ms` or `s`.
fn is_duration(text: &str) -> bool {
    let number = text.strip_suffix("ms").or_else(|| text.strip_suffix('s'));
    number.is_some_and(|digits| {
        !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
    })
}
