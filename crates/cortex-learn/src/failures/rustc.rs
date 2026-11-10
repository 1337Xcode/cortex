//! Compile errors from rustc as cargo prints them: `error[E0308]: mismatched types`, then
//! `  --> src/main.rs:12:5` for where it is, and cargo's closing `error: could not compile `calc`
//! (bin "calc") due to 2 previous errors` for each crate that failed, which gives the count.
//! cargo does not print rustc's own `aborting due to` line. Warnings are not read.
//!
//! A build of several crates prints one closing line for each crate that failed, so the counts
//! of all of them are added up. Each error is one failure with one frame, its primary location.

use super::collect::{Collector, leading_number};
use super::{CheckCounts, Failures};

/// Reads rustc's diagnostics out of the output of a build.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut expects_location = false;
    let mut error_total: Option<u32> = None;
    for line in output.lines().map(str::trim_end) {
        if let Some(rest) = line.strip_prefix("error[") {
            if let Some(code) = error_code(rest) {
                collector.set_error_kind(code);
            }
            expects_location = true;
        } else if line.starts_with("error") {
            expects_location = true;
            if let Some(count) = crate_total(line) {
                error_total = Some(error_total.unwrap_or(0).saturating_add(count));
            }
        } else if line.starts_with("warning") {
            expects_location = false;
        } else if expects_location && let Some(location) = line.trim_start().strip_prefix("--> ") {
            collector.add_located(location);
            expects_location = false;
        }
    }
    if let Some(errors) = error_total {
        collector.add_counts(CheckCounts {
            errors,
            ..CheckCounts::default()
        });
    }
    collector.finish()
}

/// The number of errors cargo's closing line for one crate gives: `error: could not compile
/// `calc` (bin "calc") due to 2 previous errors`, or, from older cargo versions, `due to previous
/// error` for one.
fn crate_total(line: &str) -> Option<u32> {
    let (_, rest) = line
        .strip_prefix("error: could not compile ")?
        .split_once(" due to ")?;
    leading_number(rest).or_else(|| rest.starts_with("previous error").then_some(1))
}

/// The error code in what follows `error[`, such as `E0308` from `E0308]: mismatched types`.
fn error_code(text: &str) -> Option<&str> {
    let (code, _) = text.split_once(']')?;
    let digits = code.strip_prefix('E')?;
    let is_code = !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit());
    is_code.then_some(code)
}
