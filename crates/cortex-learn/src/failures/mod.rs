//! What a failed test or build command's output says failed: the counts it printed, the names of
//! the tests that failed, the file and line of each stack frame it showed, and the kind of error.
//!
//! The output is read in memory only. Only names, counts, paths and line numbers leave this
//! module; no message text, code line or output is kept. [`parse_failures`] picks the parsers
//! that fit a runner by name from [`PARSERS`], and the first one that finds a failure wins. It is
//! meant for the output of a command that failed: a parser answers `None` when the output shows
//! no failure, so a run that passed gives `None` too.
//!
//! The runner names are those the command registry gives. `bun` has no row: the format of
//! `bun test` has not been read yet, so its output is not parsed (a `bun run` of another tool
//! prints that tool's own format, but the runner name cannot tell the two apart).

mod cargo_test;
mod clean;
mod collect;
mod go_test;
mod jest;
mod jvm;
mod locate;
mod node_stack;
mod paths;
mod pytest;
mod python_traceback;
mod rustc;
mod tsc;
mod unittest;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_edge;
#[cfg(test)]
mod tests_formats;

/// The most output read, in bytes: as much as an event carries of a failed command's output.
/// Summaries, failure lists and the last traceback are at the end of a run's output, so only the
/// tail is read, and the bound holds the work on output of any size.
pub(crate) const MAX_OUTPUT_BYTES: usize = cortex_core::MAX_FAILED_OUTPUT_BYTES;

/// The most failed tests named. A failure map shows a handful; a run that breaks everything
/// names the first twenty.
pub const MAX_FAILED_TESTS: usize = 20;

/// The most stack frames kept. The first frames of a traceback say where it went wrong; twenty
/// covers several tracebacks.
pub const MAX_FRAMES: usize = 20;

/// The most characters kept of a test's name or an error's kind. Parametrized tests print their
/// arguments in the name; two hundred characters keep it recognizable without keeping a payload.
pub const MAX_NAME_CHARS: usize = 200;

/// The counts a runner printed.
///
/// `failed`, `errors` and `skipped` are 0 when the runner printed none of them. `passed` is
/// `None` when the runner printed no passing count and none can be worked out from what it
/// printed: `go test` without `-v` prints only its failures, and a compiler runs no tests. It is
/// `Some(0)` when the runner prints every count it has (pytest, cargo, jest, `JUnit`) and none
/// passed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    /// Tests that failed.
    pub failed: u32,
    /// Tests that passed, when the output says.
    pub passed: Option<u32>,
    /// Tests or builds that ended in an error rather than a failed assertion.
    pub errors: u32,
    /// Tests skipped or ignored.
    pub skipped: u32,
}

/// One stack frame: a file as the output printed it and a line in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// The path as printed, with `/` separators. The caller maps it into the repository.
    pub path: String,
    /// The 1-based line.
    pub line: u32,
}

/// What a failed run's output said failed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Failures {
    /// The counts the runner printed, when it printed any.
    pub counts: Option<Counts>,
    /// The names of the failed tests, in output order, each once, at most [`MAX_FAILED_TESTS`].
    pub failed_tests: Vec<String>,
    /// Each failure's frames innermost first, failures in output order, each frame once, at most
    /// [`MAX_FRAMES`]. Frames inside installed packages, interpreter and toolchain libraries and
    /// the Rust standard library are left out.
    pub frames: Vec<Frame>,
    /// The kind of the first error: an exception class, a compiler code (`E0308`, `TS2322`).
    pub error_kind: Option<String>,
}

/// One row of the registry: the runners whose output a parser reads, and the parser.
struct Parser {
    /// The runners' names as the command registry gives them.
    runners: &'static [&'static str],
    /// Reads the (already shortened and cleaned) output; `None` when it shows no failure.
    parse: fn(&str) -> Option<Failures>,
}

/// The parsers, tried in order for a runner: the first to return `Some` wins. A parser of a
/// runner's own format comes before the generic stack-frame parser. Package managers run
/// scripts, so they try the formats of the JavaScript test and type-check tools. Python
/// tracebacks are read by the pytest and unittest parsers, which share `python_traceback`.
const PARSERS: &[Parser] = &[
    Parser {
        runners: &["pytest"],
        parse: pytest::parse,
    },
    Parser {
        runners: &["unittest"],
        parse: unittest::parse,
    },
    Parser {
        runners: &["cargo"],
        parse: cargo_test::parse,
    },
    Parser {
        runners: &["cargo"],
        parse: rustc::parse,
    },
    Parser {
        runners: &["go"],
        parse: go_test::parse,
    },
    Parser {
        runners: &["jest", "vitest", "npm", "pnpm", "yarn"],
        parse: jest::parse,
    },
    Parser {
        runners: &["tsc", "npm", "pnpm", "yarn"],
        parse: tsc::parse,
    },
    Parser {
        runners: &["jest", "vitest", "npm", "pnpm", "yarn"],
        parse: node_stack::parse,
    },
    Parser {
        runners: &["mvn", "gradle"],
        parse: jvm::parse,
    },
];

/// Reads the output of a failed command run by `runner` and says what failed, or `None` when the
/// runner has no parser or the output shows no failure. Only the last [`MAX_OUTPUT_BYTES`] of the
/// output are read, with terminal colour codes removed.
#[must_use]
pub fn parse_failures(runner: &str, output: &str) -> Option<Failures> {
    let text = clean::clean_tail(output);
    PARSERS
        .iter()
        .filter(|parser| parser.runners.contains(&runner))
        .find_map(|parser| (parser.parse)(&text))
}
