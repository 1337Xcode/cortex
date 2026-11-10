//! pytest: the summary line (`1 failed, 7 passed, 2 skipped in 0.12s`), the `FAILED` and `ERROR`
//! lines of its short summary and of verbose progress, the `path:line:` lines that end each
//! traceback block in the long and short styles, the `E` lines that name the error, and the
//! native tracebacks that `--tb=native` prints.
//!
//! Each failure's section starts at its `____ name ____` header and ends at the next `====` rule.
//! pytest prints the frames of a section outermost first, so a section is read as a trace in that
//! order. A collection error's `Traceback:` block is part of its section; the interpreter's own
//! frames in it (`importlib/__init__.py`) are dropped by the path rules.

use super::collect::{Collector, TraceOrder, apply_count, count_part, exception_name};
use super::locate::{Located, locate_prefix};
use super::paths::has_extension;
use super::python_traceback;
use super::{CheckCounts, Failures};

/// Reads pytest's output.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut counts = None;
    for line in output.lines() {
        match summary_counts(line) {
            Some(found) => counts = Some(found),
            None => read_line(&mut collector, line.trim_end()),
        }
    }
    collector.end_trace();
    if let Some(found) = counts {
        collector.add_counts(found);
    }
    python_traceback::collect(&mut collector, output);
    collector.finish()
}

/// Whether `line` is a failure section's header, `____ test_divide ____`. The `_ _ _ _` line
/// between the frames of a long-style section is not one.
fn is_section_header(line: &str) -> bool {
    line.starts_with("___") && line.ends_with("___")
}

/// The counts of a summary line, `=== 1 failed, 7 passed in 0.12s ===` or, with `-q`, the same
/// without the rule. The part after `in` must start with the time, so prose is not taken for one.
/// pytest prints only the counts that are not zero, so a missing `passed` is 0.
fn summary_counts(line: &str) -> Option<CheckCounts> {
    let body = line.trim().trim_matches('=').trim();
    let (head, time) = body.split_once(" in ")?;
    if !time.trim_start().starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let mut counts = CheckCounts {
        passed: Some(0),
        ..CheckCounts::default()
    };
    let mut found = false;
    for part in head.split(',') {
        let (number, word) = count_part(part)?;
        found |= apply_count(&mut counts, number, word);
    }
    found.then_some(counts)
}

/// Whether `name` is a test's id as pytest prints it (`tests/test_ops.py::test_divide`) or a file.
fn is_node_id(name: &str) -> bool {
    name.contains("::") || has_extension(name, "py")
}

/// Reads one line that is not a summary.
fn read_line(collector: &mut Collector, line: &str) {
    if is_section_header(line) {
        collector.begin_trace(TraceOrder::OutermostFirst);
        return;
    }
    if line.starts_with("===") {
        collector.end_trace();
        return;
    }
    if let Some(rest) = line
        .strip_prefix("FAILED ")
        .or_else(|| line.strip_prefix("ERROR "))
    {
        add_summary_entry(collector, rest);
        return;
    }
    if let Some((name, _)) = line.split_once(" FAILED ")
        && is_node_id(name)
    {
        collector.add_test(name);
        return;
    }
    if let Some(kind) = line.strip_prefix("E ").and_then(exception_name) {
        collector.set_error_kind(kind);
        return;
    }
    if let Some(found) = locate_prefix(line) {
        add_located_line(collector, &found);
    }
}

/// Adds the test of a `FAILED id - message` entry and the error the message starts with.
fn add_summary_entry(collector: &mut Collector, entry: &str) {
    let (name, message) = entry.split_once(" - ").unwrap_or((entry, ""));
    if !is_node_id(name) {
        return;
    }
    collector.add_test(name);
    if let Some(kind) = exception_name(message) {
        collector.set_error_kind(kind);
    }
}

/// Adds the frame of a `path:line: in name` line (short style) or `path:line: ErrorName` line
/// (the end of a long-style block), and in the second case the error kind. Other lines that
/// start with a location, warnings among them, are not frames.
fn add_located_line(collector: &mut Collector, found: &Located<'_>) {
    let rest = found.rest.trim();
    if rest.is_empty() || rest.starts_with("in ") {
        collector.add_frame(found.path, found.line);
    } else if let Some(kind) = exception_name(rest) {
        collector.add_frame(found.path, found.line);
        collector.set_error_kind(kind);
    }
}
