//! The TypeScript compiler's diagnostics in both of its styles: `src/ops.ts(12,5): error TS2322:
//! message` and, with `--pretty`, `src/ops.ts:12:5 - error TS2322: message`, and the diagnostics
//! that have no location (`error TS5083: Cannot read file '/repo/tsconfig.json'.`). The first
//! error's code is the error kind; the count comes from the `Found N errors` line, or from the
//! diagnostics themselves when the compiler printed none. A diagnostic with a location is one
//! failure with one frame, so frames are in output order.

use super::collect::{Collector, leading_number};
use super::locate::split_location;
use super::{Counts, Failures};

/// The text that opens a diagnostic's code.
const ERROR_MARK: &str = "error ";

/// One diagnostic line: where it is, when it says, and its code.
struct Diagnostic<'a> {
    /// The path and line, absent for a diagnostic about the configuration.
    location: Option<(&'a str, u32)>,
    /// The code, `TS2322`.
    code: &'a str,
}

/// Reads the compiler's output.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut diagnostics: u32 = 0;
    let mut found_total = None;
    for line in output.lines().map(str::trim) {
        if let Some(found) = diagnostic(line) {
            if let Some((path, number)) = found.location {
                collector.add_frame(path, number);
            }
            collector.set_error_kind(found.code);
            diagnostics = diagnostics.saturating_add(1);
        } else if let Some(rest) = line.strip_prefix("Found ")
            && rest.contains("error")
        {
            found_total = leading_number(rest);
        }
    }
    let errors = found_total.unwrap_or(diagnostics);
    collector.add_counts(Counts {
        errors,
        ..Counts::default()
    });
    collector.finish()
}

/// The diagnostic a line holds, or `None` for any other line. The code follows `error `, which
/// starts the line when the diagnostic has no location and ends the location otherwise.
fn diagnostic(line: &str) -> Option<Diagnostic<'_>> {
    let at = if line.starts_with(ERROR_MARK) {
        0
    } else {
        line.find(" error TS")? + 1
    };
    let after_mark = line.get(at + ERROR_MARK.len()..)?;
    let code = after_mark.split(':').next()?.trim();
    let digits = code.strip_prefix("TS")?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let head = line.get(..at)?.trim_end();
    if head.is_empty() {
        return Some(Diagnostic {
            location: None,
            code,
        });
    }
    let location = match head.strip_suffix(':') {
        Some(call_style) => split_call_style(call_style)?,
        None => split_location(head.strip_suffix('-')?)?,
    };
    Some(Diagnostic {
        location: Some(location),
        code,
    })
}

/// The path and line of `src/ops.ts(12,5)`.
fn split_call_style(text: &str) -> Option<(&str, u32)> {
    let (path, position) = text.strip_suffix(')')?.rsplit_once('(')?;
    let (row, _) = position.split_once(',')?;
    Some((path, row.trim().parse().ok()?))
}
