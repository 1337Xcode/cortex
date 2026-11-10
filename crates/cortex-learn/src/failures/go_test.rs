//! `go test` and `go build`: the `--- FAIL: TestName (0.00s)` lines, the `file.go:12: message`
//! lines that tests log and the compiler prints, the `panic:` line and the goroutine frames under
//! it, and the `FAIL package [build failed]` lines.
//!
//! Go prints no totals, so the counts are those of the result lines it printed, for top-level
//! tests only: a subtest prints its own indented `--- FAIL: TestDivide/by_zero` line under its
//! parent's, and counting both would count one failure twice. The failed tests are named the same
//! way, by the top-level test; its failing subtests are not named. Without `-v` Go prints no
//! `PASS`, `SKIP` or `RUN` lines, so the passing count is `None` then and `Some` once any of
//! those lines was printed.
//!
//! A panic inside a test is printed after that test's own `--- FAIL` line and is one failure with
//! it. A panic with no such line before it in its package (the binary crashed, in `init` or in
//! another goroutine) is an error. The goroutine's frames are printed innermost first.
//!
//! The frames of Go's own toolchain and of module dependencies are left out; see
//! [`is_toolchain_or_dependency`].

use super::collect::{Collector, TraceOrder};
use super::locate::{locate_prefix, split_location};
use super::paths::has_extension;
use super::{CheckCounts, Failures};

/// The marks of a trace line's program counter, ` +0x1d`, which follow the location.
const OFFSET_MARK: &str = " +0x";

/// What a run of lines has told so far.
#[derive(Default)]
struct Scan {
    /// The failed, errors and skipped counted; `passed` is kept apart, in the field below.
    counts: CheckCounts,
    /// The passing top-level tests counted.
    passed: u32,
    /// Whether a `=== RUN` or a `--- PASS` or `--- SKIP` line was seen, which `-v` prints.
    has_progress: bool,
    /// Whether a top-level `--- FAIL` was seen since the last package result line.
    failed_in_package: bool,
}

/// Reads the output of `go test` or `go build`.
pub(super) fn parse(output: &str) -> Option<Failures> {
    let mut collector = Collector::default();
    let mut scan = Scan::default();
    for line in output.lines().map(str::trim_end) {
        scan.read_line(&mut collector, line);
    }
    let counts = CheckCounts {
        passed: scan.has_progress.then_some(scan.passed),
        ..scan.counts
    };
    if counts != CheckCounts::default() {
        collector.add_counts(counts);
    }
    collector.finish()
}

impl Scan {
    /// Reads one line.
    fn read_line(&mut self, collector: &mut Collector, line: &str) {
        let trimmed = line.trim_start();
        if let Some(rest) = line.strip_prefix("--- FAIL: ") {
            self.counts.failed = self.counts.failed.saturating_add(1);
            self.failed_in_package = true;
            collector.add_test(rest.split(" (").next().unwrap_or(rest));
        } else if line.starts_with("--- PASS: ") {
            self.has_progress = true;
            self.passed = self.passed.saturating_add(1);
        } else if line.starts_with("--- SKIP: ") {
            self.has_progress = true;
            self.counts.skipped = self.counts.skipped.saturating_add(1);
        } else if line.starts_with("=== ") {
            self.has_progress = true;
        } else if trimmed.starts_with("--- ") {
            // A subtest's result: its parent's line already counts the failure.
        } else if line.starts_with("panic: ") {
            self.read_panic(collector);
        } else if line.starts_with("FAIL") && is_build_failure(line) {
            self.counts.errors = self.counts.errors.saturating_add(1);
            collector.set_error_kind("build failed");
        } else if is_package_result(line) {
            self.failed_in_package = false;
        } else if line.starts_with("goroutine ") && line.ends_with(':') {
            collector.begin_trace(TraceOrder::InnermostFirst);
        } else if line.is_empty() {
            collector.end_trace();
        } else if line.starts_with('\t') {
            add_trace_frame(collector, line);
        } else if let Some(found) = locate_prefix(trimmed)
            && has_extension(found.path, "go")
        {
            collector.add_frame(found.path, found.line);
        }
    }

    /// Reads a `panic:` line: an error unless a failed test's line came before it in the package.
    fn read_panic(&mut self, collector: &mut Collector) {
        if !self.failed_in_package {
            self.counts.errors = self.counts.errors.saturating_add(1);
        }
        collector.set_error_kind("panic");
    }
}

/// Whether a `FAIL package` line says the package did not build or set up.
fn is_build_failure(line: &str) -> bool {
    line.contains("[build failed]") || line.contains("[setup failed]")
}

/// Whether `line` is a package's result line: `ok  \tpkg\t0.1s`, `FAIL\tpkg\t0.1s` or
/// `?   \tpkg\t[no test files]`.
fn is_package_result(line: &str) -> bool {
    line.starts_with("ok ") || line.starts_with("FAIL\t") || line.starts_with("?   ")
}

/// Adds the frame of a goroutine trace line, `\t/path/file.go:9 +0x1d`: a tab, the location, and
/// perhaps the program counter's offset. The path may hold spaces, so the offset is cut off by its
/// mark and the rest is the location. Frames of the toolchain and of dependencies are dropped.
fn add_trace_frame(collector: &mut Collector, line: &str) {
    let text = line.trim_start();
    let location = text
        .split_once(OFFSET_MARK)
        .map_or(text, |(location, _)| location);
    let Some((path, number)) = split_location(location) else {
        return;
    };
    if has_extension(path, "go") && !is_toolchain_or_dependency(path) {
        collector.add_frame(path, number);
    }
}

/// Whether `path` is a file of Go's toolchain or of a module dependency, by where it lives:
///
/// - anything under a `/pkg/mod/` folder is the module cache, where dependencies and downloaded
///   toolchains (`pkg/mod/golang.org/toolchain@v0.0.1-go1.22.1.linux-amd64/src/...`) are;
/// - a `src/<package>/` folder inside a GOROOT, a folder named `go`, `go-1.22`, `go1.22.1` or
///   `libexec` (a distribution under `/usr/local/go`, `C:/Program Files/Go`, Homebrew's
///   `Cellar/go/1.22.0/libexec`), whose package path has no dot in its first element. Go's own
///   rule makes a first element without a dot the standard library's, so a GOPATH project
///   (`go/src/github.com/x/calc`) is kept, and one with a dotless name (`go/src/calc`) is not.
fn is_toolchain_or_dependency(path: &str) -> bool {
    let path = path.replace('\\', "/");
    if path.contains("/pkg/mod/") {
        return true;
    }
    let parts: Vec<&str> = path.split('/').collect();
    parts.windows(3).any(|window| {
        matches!(window, [root, src, first] if *src == "src" && is_goroot_name(root)
            && !first.contains('.'))
    })
}

/// Whether a folder name is a Go distribution's root: `go`, `go-1.22`, `go1.22.1` or `libexec`.
fn is_goroot_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "libexec"
        || lower == "go"
        || lower
            .strip_prefix("go")
            .is_some_and(|rest| rest.starts_with(|c: char| c == '-' || c.is_ascii_digit()))
}
