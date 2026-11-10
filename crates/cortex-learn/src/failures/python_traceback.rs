//! Python tracebacks, as the interpreter and the runners built on it print them: each `File`
//! line is a frame, and the unindented line that closes the block names the error.
//!
//! Python prints a traceback outermost frame first, so each one is read as a trace in that order
//! and its frames are added innermost first. A chained exception (`The above exception was the
//! direct cause of the following exception:` or `During handling of the above exception, another
//! exception occurred:` between two tracebacks) is one failure: the last exception is the one
//! that escaped, so its name is the error kind and its frames come first. The first failure's
//! kind is kept, as the other parsers do.

use super::collect::{Collector, TraceOrder, exception_name, leading_number};

/// The line that opens a traceback.
const TRACEBACK_HEADER: &str = "Traceback (most recent call last):";

/// The lines Python prints between two tracebacks of a chain.
const CHAIN_LINKS: &[&str] = &[
    "The above exception was the direct cause of the following exception:",
    "During handling of the above exception, another exception occurred:",
];

/// Where `collect` is in the output.
#[derive(Default)]
struct Scan<'a> {
    /// Whether the lines being read are inside a traceback.
    in_traceback: bool,
    /// Whether a chain's trace is open: a traceback has been read and its chain has not ended.
    chain_open: bool,
    /// Whether the line after a traceback was a chain link, so the next traceback continues it.
    linked: bool,
    /// The error that closed the latest traceback of the chain being read.
    chain_kind: Option<&'a str>,
}

/// Adds the frames and the error kind of every traceback in `output` to `collector`.
pub(super) fn collect(collector: &mut Collector, output: &str) {
    let mut scan = Scan::default();
    for line in output.lines() {
        let trimmed = line.trim_end();
        if trimmed == TRACEBACK_HEADER {
            scan.open_traceback(collector);
        } else if scan.in_traceback {
            scan.in_traceback = read_traceback_line(collector, line, &mut scan.chain_kind);
        } else if CHAIN_LINKS.contains(&trimmed) {
            scan.linked = true;
        }
    }
    scan.end_chain(collector);
}

impl Scan<'_> {
    /// Starts a traceback: the next link of the open chain, or the first of a new one.
    fn open_traceback(&mut self, collector: &mut Collector) {
        if !(self.linked && self.chain_open) {
            self.end_chain(collector);
            collector.begin_trace(TraceOrder::OutermostFirst);
            self.chain_open = true;
        }
        self.linked = false;
        self.in_traceback = true;
    }

    /// Ends the chain: its frames are added and its last error becomes the kind.
    fn end_chain(&mut self, collector: &mut Collector) {
        collector.end_trace();
        if let Some(kind) = self.chain_kind.take() {
            collector.set_error_kind(kind);
        }
        self.chain_open = false;
    }
}

/// Reads one line inside a traceback and says whether the traceback goes on. Indented lines are
/// frames and the code under them; the first unindented line is the error and ends it.
fn read_traceback_line<'a>(
    collector: &mut Collector,
    line: &'a str,
    chain_kind: &mut Option<&'a str>,
) -> bool {
    if line.starts_with([' ', '\t']) {
        if let Some((path, number)) = file_frame(line) {
            collector.add_frame(path, number);
        }
        return true;
    }
    if let Some(kind) = exception_name(line) {
        *chain_kind = Some(kind);
    }
    false
}

/// The path and line of a `  File "path", line 9, in name` line.
fn file_frame(line: &str) -> Option<(&str, u32)> {
    let rest = line.trim_start().strip_prefix("File \"")?;
    let (path, after) = rest.split_once('"')?;
    let number = after.strip_prefix(", line ").and_then(leading_number)?;
    Some((path, number))
}
