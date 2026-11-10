//! `cargo test`: the `test result:` line each test binary prints (added up over binaries), the
//! `test name ... FAILED` lines, the `---- name stdout ----` headers (a doctest's name is its
//! file, item and line) and `failures:` list, the location of each panic and the project frames
//! of the backtrace that follows it.
//!
//! A panic's location comes in two forms. The current one is `panicked at src/ops.rs:14:5:` with
//! the message on the next line. Before Rust 1.73 the message came first, quoted, and the
//! location ended it: `panicked at 'index out of bounds', src/ops.rs:20:9`, and for `assert_eq!`
//! over three lines, the location on the last (`right: `2`', src/lib.rs:5:9`).
//!
//! Each panic is one failure; its frames are the panic's location and then the backtrace's
//! project frames, which Rust prints innermost first.

use super::collect::{Collector, TraceOrder, number_before};
use super::locate::split_location;
use super::{CheckCounts, Failures};

/// How many lines after an unfinished `panicked at '` are searched for the location that ends the
/// message. A message longer than this is not a one-line assertion's; the panic then has no frame.
const MAX_MESSAGE_LINES: u32 = 40;

/// What a run of lines has told so far.
#[derive(Default)]
struct Scan {
    /// Whether the `failures:` list of test names is being read.
    in_list: bool,
    /// The lines left to search for the end of a quoted panic message.
    message_lines_left: u32,
}

/// Reads the output of `cargo test`.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut scan = Scan::default();
    for line in output.lines() {
        if scan.in_list {
            if line.trim().is_empty() {
                continue;
            }
            if line.starts_with("    ") {
                collector.add_test(line);
                continue;
            }
        }
        scan.read_line(&mut collector, line.trim_end());
    }
    collector.finish()
}

impl Scan {
    /// Reads one line outside the `failures:` list of names.
    fn read_line(&mut self, collector: &mut Collector, line: &str) {
        self.in_list = line == "failures:";
        if self.message_lines_left > 0 {
            self.read_message_line(collector, line);
        } else if let Some(summary) = line.strip_prefix("test result:") {
            collector.add_counts(result_counts(summary));
        } else if let Some(name) = failed_test_name(line).or_else(|| header_test_name(line)) {
            collector.add_test(name);
        } else if let Some((_, text)) = line.split_once(" panicked at ") {
            self.read_panic(collector, text.trim());
        } else if let Some(location) = backtrace_location(line) {
            collector.add_located(location);
        }
    }

    /// Reads the first line of a panic, after `panicked at`, and opens its trace.
    fn read_panic(&mut self, collector: &mut Collector, text: &str) {
        collector.begin_trace(TraceOrder::InnermostFirst);
        if !text.starts_with(['\'', '"']) {
            collector.add_located(text);
        } else if let Some(location) = closing_location(text) {
            collector.add_located(location);
        } else {
            self.message_lines_left = MAX_MESSAGE_LINES;
        }
    }

    /// Reads a line of a quoted panic message that has not ended yet: the line that ends in the
    /// location ends it, and a header that starts something else gives up.
    fn read_message_line(&mut self, collector: &mut Collector, line: &str) {
        self.message_lines_left -= 1;
        if let Some(location) = closing_location(line) {
            collector.add_located(location);
            self.message_lines_left = 0;
        } else if line.starts_with("---- ") || line == "failures:" {
            self.message_lines_left = 0;
        }
    }
}

/// The counts of a `test result: FAILED. 3 passed; 1 failed; 0 ignored; ...` line, after the
/// colon. A count the line lacks is absent: `passed` stays `None`.
fn result_counts(summary: &str) -> CheckCounts {
    CheckCounts {
        failed: number_before(summary, "failed").unwrap_or(0),
        passed: number_before(summary, "passed"),
        errors: 0,
        skipped: number_before(summary, "ignored").unwrap_or(0),
    }
}

/// The name in a `test name ... FAILED` line.
fn failed_test_name(line: &str) -> Option<&str> {
    line.strip_prefix("test ")?.strip_suffix(" ... FAILED")
}

/// The name in a `---- name stdout ----` header, which also comes as `stderr`.
fn header_test_name(line: &str) -> Option<&str> {
    let inner = line.strip_prefix("---- ")?;
    inner
        .strip_suffix(" stdout ----")
        .or_else(|| inner.strip_suffix(" stderr ----"))
}

/// The location that ends a quoted panic message, `', src/ops.rs:20:9` (or with a double quote),
/// in `text`, if `text` ends so. What follows the last closing quote and comma must be a location,
/// so a message that holds a quote and a comma is not cut short.
fn closing_location(text: &str) -> Option<&str> {
    let (_, location) = text
        .rsplit_once("', ")
        .or_else(|| text.rsplit_once("\", "))?;
    split_location(location).map(|_| location)
}

/// The location of a backtrace line, `             at ./src/ops.rs:14:5`, as a path without
/// the `./` that the backtrace puts in front of the project's own files.
fn backtrace_location(line: &str) -> Option<&str> {
    if !line.starts_with(' ') {
        return None;
    }
    let location = line.trim_start().strip_prefix("at ")?;
    Some(location.strip_prefix("./").unwrap_or(location))
}
