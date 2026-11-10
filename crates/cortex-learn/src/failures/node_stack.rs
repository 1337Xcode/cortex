//! Node stack frames, `    at divide (src/ops.js:14:11)` and `    at src/ops.js:14:11`, which
//! Node, jest, vitest and every JavaScript test runner print. Frames in `node_modules` and
//! Node's own `node:` modules are dropped when they are added. The generic parser reads frames
//! that follow an error line; the runners' parsers use [`frame_in`] for the frames of their own
//! failure blocks. Node prints the innermost frame first, so each error's frames are one trace in
//! that order.

use super::Failures;
use super::collect::{Collector, TraceOrder, exception_name, is_error_type};
use super::locate::split_location;

/// Reads the output as an uncaught error and its stack: the frames after the first error line.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut seen_error = false;
    for line in output.lines() {
        if let Some((path, number)) = frame_in(line) {
            if seen_error {
                collector.add_frame(path, number);
            }
        } else if let Some(kind) = error_line(line) {
            collector.set_error_kind(kind);
            collector.begin_trace(TraceOrder::InnermostFirst);
            seen_error = true;
        }
    }
    collector.finish()
}

/// The path and line of a stack frame line, or `None` for any other line, and for a frame with no
/// file and line (`at async Promise.all (index 0)`). A `file://` address becomes a path.
pub(super) fn frame_in(line: &str) -> Option<(&str, u32)> {
    if !line.starts_with([' ', '\t']) {
        return None;
    }
    let rest = line.trim_start().strip_prefix("at ")?;
    let rest = rest.strip_prefix("async ").unwrap_or(rest).trim_end();
    let location = match rest.strip_suffix(')') {
        Some(inner) => inner
            .rsplit_once(" (")
            .map_or(inner, |(_, location)| location)
            .trim_start_matches('('),
        None => rest,
    };
    let (path, number) = split_location(location)?;
    Some((strip_file_scheme(path), number))
}

/// The path of a `file:///x/y.js` address, with the slash before a Windows drive letter removed.
pub(super) fn strip_file_scheme(address: &str) -> &str {
    let Some(rest) = address.strip_prefix("file://") else {
        return address;
    };
    match rest.strip_prefix('/') {
        Some(with_drive) if with_drive.chars().nth(1) == Some(':') => with_drive,
        _ => rest,
    }
}

/// The error type an error line starts with, such as `TypeError` in `TypeError: x is not a
/// function` or in `AssertionError [ERR_ASSERTION]: x`; the line is not indented.
fn error_line(line: &str) -> Option<&str> {
    if line.starts_with(char::is_whitespace) {
        return None;
    }
    let name = line.split([':', ' ']).next()?;
    let rest = line.get(name.len()..)?;
    let is_header = rest.starts_with(':') || rest.starts_with(" [");
    if !is_header || !is_error_type(name) {
        return None;
    }
    exception_name(name)
}
