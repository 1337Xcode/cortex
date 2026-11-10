//! `JUnit` as Maven's surefire and Gradle print it: the `Tests run: 8, Failures: 1, Errors: 0,
//! Skipped: 0` totals (Gradle's `8 tests completed, 1 failed`), the failed test's line, the
//! `\tat com.x.Ops.divide(Ops.java:14)` frames, the error's type and the compiler errors of a
//! build that did not compile (javac's, Maven's and Kotlin's `e: file:///x/Ops.kt:12:5 message`).
//!
//! A frame names a class, not a file, so its path is rebuilt from the package folders and the
//! file name the frame gives: `com/x/Ops.java`. Frames of the JDK, `JUnit` and the build tools
//! are dropped. The JVM prints a trace innermost first, so the frames of each failed test are one
//! trace in that order.
//!
//! The totals are the last `Tests run:` line that has no class suffix: surefire ends a per-class
//! line with ` - in com.x.FooTest` (surefire 2) or ` -- in com.x.FooTest` (surefire 3), and the
//! final totals line has neither. With no totals line there are no test counts.

use super::collect::{
    Collector, TraceOrder, exception_name, is_error_type, number_after, number_before, take_digits,
};
use super::locate::{locate_prefix, split_location};
use super::node_stack::strip_file_scheme;
use super::paths::has_extension;
use super::{Counts, Failures};

/// Folders, as paths, of the frames that are the platform's or the test tools', not the project's.
const FRAMEWORK_PREFIXES: &[&str] = &[
    "java/",
    "javax/",
    "jdk/",
    "sun/",
    "com/sun/",
    "junit/",
    "org/junit/",
    "org/opentest4j/",
    "org/apache/maven/",
    "org/apache/tools/ant/",
    "org/gradle/",
    "org/testng/",
];

/// What the totals said.
#[derive(Default)]
struct Totals {
    /// The test totals, from the last totals line.
    tests: Option<Counts>,
    /// The compiler's count of errors, from its `2 errors` line.
    build_errors: u32,
    /// The compiler errors printed one by one, which count when there is no `2 errors` line
    /// (Kotlin prints none).
    printed_errors: u32,
}

/// Reads the output of Maven or Gradle.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut totals = Totals::default();
    for line in output.lines().map(str::trim_end) {
        read_line(&mut collector, &mut totals, strip_level(line));
    }
    if let Some(tests) = totals.tests {
        collector.add_counts(tests);
    }
    let errors = if totals.build_errors > 0 {
        totals.build_errors
    } else {
        totals.printed_errors
    };
    if errors > 0 {
        collector.add_counts(Counts {
            errors,
            ..Counts::default()
        });
    }
    collector.finish()
}

/// `line` without Maven's `[ERROR] `, `[WARNING] ` or `[INFO] ` prefix.
fn strip_level(line: &str) -> &str {
    line.strip_prefix('[')
        .and_then(|rest| rest.split_once("] "))
        .filter(|(level, _)| matches!(*level, "ERROR" | "WARNING" | "INFO"))
        .map_or(line, |(_, body)| body)
}

/// Reads one line, its level prefix removed.
fn read_line(collector: &mut Collector, totals: &mut Totals, body: &str) {
    let text = body.trim();
    if let Some(found) = maven_totals(text).or_else(|| gradle_totals(text)) {
        totals.tests = Some(found);
    } else if let Some(count) = build_error_count(text) {
        totals.build_errors = count;
    } else if let Some(name) = failed_test_name(text) {
        collector.add_test(name);
        collector.begin_trace(TraceOrder::InnermostFirst);
    } else if let Some((path, number)) = stack_frame(text) {
        collector.add_frame(&path, number);
    } else if let Some((kind, path, number)) = gradle_trace(text) {
        collector.set_error_kind(kind);
        collector.add_frame(path, number);
    } else if let Some((path, number)) = kotlin_error(text).or_else(|| compile_location(text)) {
        collector.add_frame(path, number);
        totals.printed_errors = totals.printed_errors.saturating_add(1);
    } else if !body.starts_with(char::is_whitespace)
        && let Some(kind) = error_type(text)
    {
        collector.set_error_kind(kind);
    }
}

/// The counts of surefire's totals line. The per-class lines, which end with ` - in Class` or
/// ` -- in Class` or carry `<<<`, are not the totals.
fn maven_totals(text: &str) -> Option<Counts> {
    let has_class = [" - in ", " -- in ", "<<<"]
        .iter()
        .any(|mark| text.contains(mark));
    if !text.starts_with("Tests run:") || has_class {
        return None;
    }
    let run = number_after(text, "Tests run:")?;
    let failed = number_after(text, "Failures:").unwrap_or(0);
    let errors = number_after(text, "Errors:").unwrap_or(0);
    let skipped = number_after(text, "Skipped:").unwrap_or(0);
    Some(counts_from_run(run, failed, errors, skipped))
}

/// The counts of Gradle's `8 tests completed, 1 failed, 2 skipped` line.
fn gradle_totals(text: &str) -> Option<Counts> {
    let (run, rest) = take_digits(text)?;
    if !rest.trim_start().starts_with("test") || !rest.contains("completed") {
        return None;
    }
    let failed = number_before(rest, "failed").unwrap_or(0);
    let skipped = number_before(rest, "skipped").unwrap_or(0);
    Some(counts_from_run(run, failed, 0, skipped))
}

/// The counts of a run of `run` tests: those that did not fail, err or skip passed.
fn counts_from_run(run: u32, failed: u32, errors: u32, skipped: u32) -> Counts {
    let not_passed = failed.saturating_add(errors).saturating_add(skipped);
    Counts {
        failed,
        passed: Some(run.saturating_sub(not_passed)),
        errors,
        skipped,
    }
}

/// The number in the compiler's `2 errors` line.
fn build_error_count(text: &str) -> Option<u32> {
    let (count, rest) = take_digits(text)?;
    matches!(rest, " error" | " errors").then_some(count)
}

/// The name of a failed test: surefire's `divide  Time elapsed: 0.01 s  <<< FAILURE!` (or
/// `<<< ERROR!`) line, and Gradle's `OpsTest > divide() FAILED`.
fn failed_test_name(text: &str) -> Option<&str> {
    if let Some((head, _)) = text.split_once(" Time elapsed")
        && !text.starts_with("Tests run:")
        && (text.contains("<<< FAILURE!") || text.contains("<<< ERROR!"))
    {
        return Some(head.trim_end().trim_end_matches('-').trim_end());
    }
    let name = text.strip_suffix(" FAILED")?;
    name.contains(" > ").then_some(name)
}

/// The path and line of a stack frame, `at com.x.Ops.divide(Ops.java:14)`, rebuilt as
/// `com/x/Ops.java`. `None` for other lines, for frames with no line (`Native Method`) and for
/// the frames of [`FRAMEWORK_PREFIXES`].
fn stack_frame(text: &str) -> Option<(String, u32)> {
    let rest = text.strip_prefix("at ")?;
    let (qualified, location) = rest.split_once('(')?;
    let (file, line_text) = location.strip_suffix(')')?.rsplit_once(':')?;
    let number: u32 = line_text.parse().ok()?;
    // A module prefix, `java.base/`, comes before the class path.
    let method_path = qualified.rsplit('/').next()?;
    let mut parts: Vec<&str> = method_path.split('.').collect();
    parts.pop()?;
    parts.pop()?;
    let path = if parts.is_empty() {
        file.to_owned()
    } else {
        let folders = parts.join("/");
        format!("{folders}/{file}")
    };
    let is_framework = FRAMEWORK_PREFIXES
        .iter()
        .any(|prefix| path.starts_with(prefix));
    (!is_framework).then_some((path, number))
}

/// The error type, path and line of Gradle's `java.lang.AssertionError at OpsTest.java:14` line.
fn gradle_trace(text: &str) -> Option<(&str, &str, u32)> {
    let (kind_text, location) = text.split_once(" at ")?;
    let kind = error_type(kind_text)?;
    let (path, number) = split_location(location)?;
    Some((kind, path, number))
}

/// The path and line of a compiler error: Maven's `/x/Ops.java:[12,5] cannot find symbol` and
/// javac's `/x/Ops.java:12: error: cannot find symbol`.
fn compile_location(text: &str) -> Option<(&str, u32)> {
    let is_source = |path: &str| has_extension(path, "java") || has_extension(path, "kt");
    if let Some((path, rest)) = text.split_once(":[")
        && is_source(path)
    {
        let row = take_digits(rest)?.0;
        return Some((path, row));
    }
    let found = locate_prefix(text)?;
    let is_error = found.rest.trim_start().starts_with("error:");
    (is_source(found.path) && is_error).then_some((found.path, found.line))
}

/// The path and line of a Kotlin compiler error, `e: file:///x/Ops.kt:12:5 Unresolved reference:
/// foo` or, from newer compilers, `... Unresolved reference 'foo'.`. The `file://` address becomes
/// a path.
fn kotlin_error(text: &str) -> Option<(&str, u32)> {
    let address = strip_file_scheme(text.strip_prefix("e: ")?);
    let end = address.find(".kt:")?;
    let path = address.get(..end + ".kt".len())?;
    let number = take_digits(address.get(end + ".kt:".len()..)?)?.0;
    Some((path, number))
}

/// The type of the error a line starts with, `java.lang.AssertionError` in
/// `java.lang.AssertionError: expected:<1> but was:<2>`.
fn error_type(text: &str) -> Option<&str> {
    exception_name(text).filter(|name| is_error_type(name))
}
