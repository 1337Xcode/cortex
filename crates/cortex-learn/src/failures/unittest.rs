//! Python's unittest: the `FAIL:` and `ERROR:` headers of each failed test, the `Ran N tests`
//! line and the `FAILED (failures=1, errors=1)` line that ends the run, and the tracebacks under
//! the headers.
//!
//! The parenthesised parts of the closing line are read by their exact key (`failures`, `errors`,
//! `skipped`, `expected failures`, `unexpected successes`). unittest reports a run as FAILED when
//! a test succeeded that was marked as an expected failure, so each unexpected success is counted
//! as a failed test. An expected failure is a test that failed as it was marked to: it does not
//! fail the run and unittest counts it as run, so it stays among the passed ones.

use super::collect::{Collector, leading_number};
use super::python_traceback;
use super::{Counts, Failures};

/// What the closing `FAILED (...)` line said, by key.
#[derive(Default)]
struct Outcome {
    /// `failures=`: tests whose assertion failed.
    failures: u32,
    /// `errors=`: tests that raised.
    errors: u32,
    /// `skipped=`.
    skipped: u32,
    /// `unexpected successes=`: tests marked as expected failures that passed.
    unexpected_successes: u32,
}

/// Reads unittest's output.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut ran = None;
    let mut outcome = None;
    for line in output.lines().map(str::trim_end) {
        if let Some(name) = line
            .strip_prefix("FAIL: ")
            .or_else(|| line.strip_prefix("ERROR: "))
        {
            collector.add_test(name);
        } else if let Some(rest) = line.strip_prefix("Ran ") {
            ran = leading_number(rest);
        } else if let Some(rest) = line.strip_prefix("FAILED (") {
            outcome = Some(read_outcome(rest));
        }
    }
    if let Some(found) = outcome {
        collector.add_counts(counts_of(&found, ran));
    }
    python_traceback::collect(&mut collector, output);
    collector.finish()
}

/// The parts of a `FAILED (failures=1, errors=1, expected failures=2)` line, given what follows
/// its opening parenthesis. Each part is a `key=value` split at the comma; the key is matched
/// whole, so `expected failures=` is not `failures=`. Unknown keys are ignored.
fn read_outcome(rest: &str) -> Outcome {
    let mut outcome = Outcome::default();
    let inside = rest.trim_end().trim_end_matches(')');
    for part in inside.split(',') {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let Some(number) = leading_number(value) else {
            continue;
        };
        match key.trim() {
            "failures" => outcome.failures = number,
            "errors" => outcome.errors = number,
            "skipped" => outcome.skipped = number,
            "unexpected successes" => outcome.unexpected_successes = number,
            _ => {}
        }
    }
    outcome
}

/// The counts of a run of `ran` tests, when the `Ran` line was there: those that did not fail,
/// err, skip or succeed unexpectedly passed, expected failures among them.
fn counts_of(outcome: &Outcome, ran: Option<u32>) -> Counts {
    let failed = outcome
        .failures
        .saturating_add(outcome.unexpected_successes);
    let not_passed = failed
        .saturating_add(outcome.errors)
        .saturating_add(outcome.skipped);
    Counts {
        failed,
        passed: ran.map(|ran| ran.saturating_sub(not_passed)),
        errors: outcome.errors,
        skipped: outcome.skipped,
    }
}
