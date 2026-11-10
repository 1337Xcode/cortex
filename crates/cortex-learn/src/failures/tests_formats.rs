//! The formats the review found uncovered or wrong, each on a small fixture of real output for
//! the version named in its comment, with the findings counted by hand.

use super::tests::{counts, counts_without_passed, frames, joined, kind, names, parsed};
use super::*;

/// unittest, one test that raised and one expected failure, four ran: the run is
/// `FAILED (errors=1, expected failures=1)` (`CPython` 3.12 `unittest/runner.py` writes each count
/// under its key).
/// `expected failures=1` is not a failure: 0 failed, 1 error, and the other three of the four
/// that ran passed (the expected failure among them).
const UNITTEST_EXPECTED_FAILURE: &str = r#"
.xE.
======================================================================
ERROR: test_parse (tests.test_ops.TestOps.test_parse)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "/repo/tests/test_ops.py", line 20, in test_parse
    parse("x")
ValueError: bad

----------------------------------------------------------------------
Ran 4 tests in 0.002s

FAILED (errors=1, expected failures=1)
"#;

/// An expected failure is not counted as a failure.
#[test]
fn unittest_expected_failures_are_not_failures() {
    assert_eq!(
        parsed("unittest", UNITTEST_EXPECTED_FAILURE),
        Failures {
            counts: counts(0, 3, 1, 0),
            failed_tests: names(&["test_parse (tests.test_ops.TestOps.test_parse)"]),
            frames: frames(&[("/repo/tests/test_ops.py", 20)]),
            error_kind: kind("ValueError"),
        }
    );
}

/// A test marked as an expected failure that passed makes the run FAILED: of 3 that ran, 1 is
/// counted failed and 2 passed.
#[test]
fn unittest_unexpected_successes_are_failures() {
    let output = "Ran 3 tests in 0.001s\n\nFAILED (unexpected successes=1)\n";
    assert_eq!(parsed("unittest", output).counts, counts(1, 2, 0, 0));
}

/// cargo test before Rust 1.73, the three-line `assert_eq!` panic whose location ends its last
/// line; the output is the example in Rust By Example's unit testing page, which has the shape.
const CARGO_OLD_ASSERT: &str = r"
running 2 tests
test tests::test_bad_add ... FAILED
test tests::test_add ... ok

failures:

---- tests::test_bad_add stdout ----
        thread 'tests::test_bad_add' panicked at 'assertion failed: `(left == right)`
  left: `-1`,
 right: `3`', src/lib.rs:21:8
note: Run with `RUST_BACKTRACE=1` for a backtrace.

failures:
    tests::test_bad_add

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
";

/// The location on the last line of the old multi-line panic is the frame.
#[test]
fn cargo_old_multi_line_panic_gives_the_location_on_its_last_line() {
    assert_eq!(
        parsed("cargo", CARGO_OLD_ASSERT),
        Failures {
            counts: counts(1, 1, 0, 0),
            failed_tests: names(&["tests::test_bad_add"]),
            frames: frames(&[("src/lib.rs", 21)]),
            error_kind: None,
        }
    );
}

/// A doctest is named by its file, item and line in the `FAILED` line, the section header and
/// the list, which hold spaces and parentheses; it counts once.
const CARGO_DOCTEST: &str = r"
running 1 test
test src/lib.rs - add (line 3) ... FAILED

failures:

---- src/lib.rs - add (line 3) stdout ----
Test executable failed (exit status: 101).

failures:
    src/lib.rs - add (line 3)

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.15s
";

/// A doctest failure is named once.
#[test]
fn cargo_doctest_failure_is_named_by_file_item_and_line() {
    assert_eq!(
        parsed("cargo", CARGO_DOCTEST),
        Failures {
            counts: counts(1, 0, 0, 0),
            failed_tests: names(&["src/lib.rs - add (line 3)"]),
            frames: Vec::new(),
            error_kind: None,
        }
    );
}

/// A panic with a backtrace (`RUST_BACKTRACE=1`): the panic's location, then the backtrace's
/// frames innermost first. The three standard library frames are dropped, `./` is taken off the
/// project's, and the frame at `src/ops.rs:14` is in the result once.
const CARGO_BACKTRACE: &str = r"
running 1 test
test ops::divide ... FAILED

failures:

---- ops::divide stdout ----
thread 'ops::divide' panicked at src/ops.rs:14:5:
attempt to divide by zero
stack backtrace:
   0: rust_begin_unwind
             at /rustc/abc123/library/std/src/panicking.rs:645:5
   1: core::panicking::panic_fmt
             at /rustc/abc123/library/core/src/panicking.rs:72:14
   2: core::panicking::panic
             at /rustc/abc123/library/core/src/panicking.rs:144:5
   3: calc::ops::divide
             at ./src/ops.rs:14:5
   4: calc::ops::divide::{{closure}}
             at ./src/ops.rs:30:9
note: Some details are omitted, run with `RUST_BACKTRACE=full` for a verbose backtrace.

failures:
    ops::divide

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";

/// The backtrace's frames follow the panic's, innermost first.
#[test]
fn cargo_backtrace_frames_are_innermost_first() {
    assert_eq!(
        parsed("cargo", CARGO_BACKTRACE).frames,
        frames(&[("src/ops.rs", 14), ("src/ops.rs", 30)])
    );
}

/// Two crates of a workspace fail: cargo prints a closing line for each, 1 error and 2 errors, so
/// the count is 3. The errors' locations are the frames.
const CARGO_TWO_CRATES: &str = r#"
error[E0308]: mismatched types
 --> a/src/lib.rs:2:5
  |
2 |     "x"
  |     ^^^ expected `u32`, found `&str`

error: could not compile `a` (lib) due to 1 previous error
error[E0425]: cannot find value `y` in this scope
 --> b/src/main.rs:1:13
  |
1 | fn main() { y }
  |             ^ not found in this scope

error[E0425]: cannot find value `z` in this scope
 --> b/src/main.rs:4:5
  |
4 |     z
  |     ^ not found in this scope

error: could not compile `b` (bin "b") due to 2 previous errors
"#;

/// The closing lines of several crates are added up.
#[test]
fn cargo_closing_lines_of_several_crates_are_added_up() {
    assert_eq!(
        parsed("cargo", CARGO_TWO_CRATES),
        Failures {
            counts: counts_without_passed(0, 3, 0),
            failed_tests: Vec::new(),
            frames: frames(&[
                ("a/src/lib.rs", 2),
                ("b/src/main.rs", 1),
                ("b/src/main.rs", 4)
            ]),
            error_kind: kind("E0308"),
        }
    );
}

/// Older cargo versions closed a single error with `due to previous error`, with no number.
#[test]
fn cargo_closing_line_without_a_number_is_one_error() {
    let output = "error[E0308]: mismatched types\n --> src/main.rs:2:18\n\
                  error: could not compile `calc` due to previous error\n";
    assert_eq!(
        parsed("cargo", output).counts,
        counts_without_passed(0, 1, 0)
    );
}

/// pytest with `--tb=native`: the section holds a standard library traceback, outermost frame
/// first. Its frames come out innermost first and the error is named by the traceback's last line.
const PYTEST_NATIVE: &str = r#"
============================= test session starts ==============================
collected 1 item

tests/test_ops.py F                                                      [100%]

=================================== FAILURES ===================================
________________________________ test_divide ___________________________________
Traceback (most recent call last):
  File "/repo/tests/test_ops.py", line 14, in test_divide
    assert divide(1, 0) == 0
  File "/repo/calc/ops.py", line 9, in divide
    return a / b
ZeroDivisionError: division by zero
=========================== short test summary info ============================
FAILED tests/test_ops.py::test_divide - ZeroDivisionError: division by zero
============================== 1 failed in 0.03s ===============================
"#;

/// The native traceback gives its frames innermost first.
#[test]
fn pytest_native_traceback_gives_frames_innermost_first() {
    assert_eq!(
        parsed("pytest", PYTEST_NATIVE),
        Failures {
            counts: counts(1, 0, 0, 0),
            failed_tests: names(&["tests/test_ops.py::test_divide"]),
            frames: frames(&[("/repo/calc/ops.py", 9), ("/repo/tests/test_ops.py", 14)]),
            error_kind: kind("ZeroDivisionError"),
        }
    );
}

/// pytest's short style with two failed tests of two frames each. Each section is printed
/// outermost first and is reversed on its own: `calc/a.py` then `tests/test_a.py`, and then the
/// second test's two.
const PYTEST_TWO_FAILURES: &str = r"
=================================== FAILURES ===================================
__________________________________ test_a ______________________________________
tests/test_a.py:5: in test_a
    a()
calc/a.py:3: in a
    raise ValueError
E   ValueError
__________________________________ test_b ______________________________________
tests/test_b.py:7: in test_b
    b()
calc/b.py:4: in b
    raise KeyError
E   KeyError
=========================== short test summary info ============================
FAILED tests/test_a.py::test_a - ValueError
FAILED tests/test_b.py::test_b - KeyError
============================== 2 failed in 0.02s ===============================
";

/// Each failure's frames are reversed on their own.
#[test]
fn pytest_reverses_each_failures_frames_on_its_own() {
    assert_eq!(
        parsed("pytest", PYTEST_TWO_FAILURES).frames,
        frames(&[
            ("calc/a.py", 3),
            ("tests/test_a.py", 5),
            ("calc/b.py", 4),
            ("tests/test_b.py", 7),
        ])
    );
}

/// Surefire 3's per-class line for a class with a failure.
const SUREFIRE_3_FAILED_CLASS: &str = concat!(
    "[ERROR] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0, ",
    "Time elapsed: 0.05 s <<< FAILURE! -- in com.x.OpsTest"
);

/// Surefire 3 (the `-- in` and `-- Time elapsed` shapes of maven-surefire issues 834 and 3303):
/// the per-class line ends `-- in com.x.OpsTest`, and the totals are the last line, 8 run and 1
/// failed, so 7 passed. The two project frames are in the order the JVM printed them.
#[test]
fn surefire_3_totals_are_the_line_without_a_class() {
    let output = joined(&[
        "[INFO] Running com.x.OpsTest",
        SUREFIRE_3_FAILED_CLASS,
        "[ERROR] com.x.OpsTest.divide -- Time elapsed: 0.01 s <<< FAILURE!",
        "java.lang.AssertionError: expected:<1> but was:<2>",
        "\tat org.junit.Assert.fail(Assert.java:89)",
        "\tat com.x.Ops.divide(Ops.java:9)",
        "\tat com.x.OpsTest.divide(OpsTest.java:14)",
        "",
        "[INFO] Results:",
        "[ERROR] Failures: ",
        "[ERROR]   OpsTest.divide:14 expected:<1> but was:<2>",
        "[INFO] ",
        "[ERROR] Tests run: 8, Failures: 1, Errors: 0, Skipped: 0",
    ]);
    assert_eq!(
        parsed("mvn", &output),
        Failures {
            counts: counts(1, 7, 0, 0),
            failed_tests: names(&["com.x.OpsTest.divide"]),
            frames: frames(&[("com/x/Ops.java", 9), ("com/x/OpsTest.java", 14)]),
            error_kind: kind("java.lang.AssertionError"),
        }
    );
}

/// A build cut off before surefire's totals line has only per-class lines, and the last of them,
/// a class that passed, is not the totals: there are no counts, and the failed test and its
/// frame are still found.
#[test]
fn surefire_3_per_class_lines_are_not_totals() {
    let output = joined(&[
        SUREFIRE_3_FAILED_CLASS,
        "[ERROR] com.x.OpsTest.divide -- Time elapsed: 0.01 s <<< FAILURE!",
        "java.lang.AssertionError: expected:<1> but was:<2>",
        "\tat com.x.OpsTest.divide(OpsTest.java:14)",
        "",
        concat!(
            "[INFO] Tests run: 2, Failures: 0, Errors: 0, Skipped: 0, ",
            "Time elapsed: 0.01 s -- in com.x.OtherTest"
        ),
    ]);
    assert_eq!(
        parsed("mvn", &output),
        Failures {
            counts: None,
            failed_tests: names(&["com.x.OpsTest.divide"]),
            frames: frames(&[("com/x/OpsTest.java", 14)]),
            error_kind: kind("java.lang.AssertionError"),
        }
    );
}

/// Kotlin compile errors as Gradle prints them (the `e: file:///...` shape of the Gradle forum
/// thread 50532): the older `Unresolved reference: foo` and the newer
/// `Unresolved reference 'bar'.` The `file://` is removed, the two errors are counted, and a
/// Windows drive address keeps its drive letter without the slash before it.
#[test]
fn kotlin_compile_errors_give_locations_and_a_count() {
    let output = joined(&[
        "> Task :compileKotlin FAILED",
        "e: file:///work/calc/src/main/kotlin/com/x/Ops.kt:12:5 Unresolved reference: foo",
        "e: file:///work/calc/src/main/kotlin/com/x/Ops.kt:20:9 Unresolved reference 'bar'.",
        "",
        "FAILURE: Build failed with an exception.",
    ]);
    assert_eq!(
        parsed("gradle", &output),
        Failures {
            counts: counts_without_passed(0, 2, 0),
            failed_tests: Vec::new(),
            frames: frames(&[
                ("/work/calc/src/main/kotlin/com/x/Ops.kt", 12),
                ("/work/calc/src/main/kotlin/com/x/Ops.kt", 20),
            ]),
            error_kind: None,
        }
    );
    let windows = "e: file:///D:/CODE/app/src/main/kotlin/Ops.kt:52:20 Unresolved reference: x";
    assert_eq!(
        parsed("gradle", windows).frames,
        frames(&[("D:/CODE/app/src/main/kotlin/Ops.kt", 52)])
    );
}

/// `go test -v` with a failing subtest: the parent prints `--- FAIL: TestDivide` and the subtest
/// an indented `--- FAIL: TestDivide/by_zero`. The top-level tests are `TestDivide` (failed) and
/// `TestAdd` (passed), so 1 failed and 1 passed; only the parent is named.
#[test]
fn go_subtests_are_not_counted_or_named_besides_their_parent() {
    let output = joined(&[
        "=== RUN   TestDivide",
        "=== RUN   TestDivide/by_zero",
        "    ops_test.go:15: expected error",
        "=== RUN   TestDivide/by_one",
        "--- FAIL: TestDivide (0.00s)",
        "    --- FAIL: TestDivide/by_zero (0.00s)",
        "    --- PASS: TestDivide/by_one (0.00s)",
        "=== RUN   TestAdd",
        "--- PASS: TestAdd (0.00s)",
        "FAIL",
        "exit status 1",
        "FAIL\tgithub.com/x/calc\t0.004s",
    ]);
    assert_eq!(
        parsed("go", &output),
        Failures {
            counts: counts(1, 1, 0, 0),
            failed_tests: names(&["TestDivide"]),
            frames: frames(&[("ops_test.go", 15)]),
            error_kind: None,
        }
    );
}

/// With `-v` and no test passing the count is 0; without `-v` it is not printed, so `None`.
#[test]
fn go_passing_count_is_zero_with_v_and_absent_without_it() {
    let end = "FAIL\nFAIL\tgithub.com/x/calc\t0.1s\n";
    let verbose = format!("=== RUN   TestA\n--- FAIL: TestA (0.00s)\n{end}");
    assert_eq!(parsed("go", &verbose).counts, counts(1, 0, 0, 0));
    let quiet = format!("--- FAIL: TestA (0.00s)\n{end}");
    assert_eq!(parsed("go", &quiet).counts, counts_without_passed(1, 0, 0));
}

/// A panic with no `--- FAIL` line before it in its package, as when `init` panics, is the
/// binary crashing: one error, with the goroutine's frame.
#[test]
fn go_panic_with_no_failed_test_is_an_error() {
    let output = joined(&[
        "panic: boom",
        "",
        "goroutine 1 [running]:",
        "main.init.0()",
        "\t/work/calc/init.go:7 +0x25",
        "exit status 2",
        "FAIL\tgithub.com/x/calc\t0.002s",
    ]);
    assert_eq!(
        parsed("go", &output),
        Failures {
            counts: counts_without_passed(0, 1, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("/work/calc/init.go", 7)]),
            error_kind: kind("panic"),
        }
    );
}

/// A trace from Homebrew's Go (`Cellar/go/1.22.0/libexec/src`, as in the traces of the Learn Go
/// With Tests concurrency chapter, Go 1.9.3 there), a Windows install, a Debian one and a
/// downloaded toolchain, with a dependency in the module cache. Only the project's frames stay,
/// in the order printed: `/work/calc/ops.go`, a GOPATH-mode project (`go/src/github.com/...`
/// has a dot in its first element), and a project under a path with spaces, whose line ends
/// ` +0x1d`.
#[test]
fn go_trace_keeps_the_projects_frames_under_any_toolchain() {
    let output = joined(&[
        "--- FAIL: TestIndex (0.00s)",
        "panic: runtime error: index out of range [5] with length 3 [recovered]",
        "\tpanic: runtime error: index out of range [5] with length 3",
        "",
        "goroutine 7 [running]:",
        "testing.tRunner.func1.2({0x1, 0x2})",
        "\t/opt/homebrew/Cellar/go/1.22.0/libexec/src/testing/testing.go:1631 +0x24a",
        "panic({0x1, 0x2})",
        "\t/usr/lib/go-1.22/src/runtime/panic.go:770 +0x132",
        "github.com/x/calc.Index(...)",
        "\t/work/calc/ops.go:9",
        "github.com/x/calc.Sum(...)",
        "\t/home/u/go/src/github.com/x/calc/sum.go:4 +0x2",
        "github.com/y/dep.Do(...)",
        "\t/home/u/go/pkg/mod/github.com/y/dep@v1.2.0/dep.go:30",
        "github.com/x/calc.TestIndex(0xc000102000?)",
        "\tC:/Users/John Smith/calc/ops_test.go:18 +0x1d",
        "testing.tRunner(0xc0001, 0x1)",
        concat!(
            "\t/home/u/go/pkg/mod/golang.org/toolchain@v0.0.1-go1.22.1.linux-amd64",
            "/src/testing/testing.go:1689 +0xfb"
        ),
        "testing.tRunner(0xc0001, 0x1)",
        "\tC:/Program Files/Go/src/testing/testing.go:1689 +0xfb",
        "FAIL\tgithub.com/x/calc\t0.015s",
    ]);
    assert_eq!(
        parsed("go", &output).frames,
        frames(&[
            ("/work/calc/ops.go", 9),
            ("/home/u/go/src/github.com/x/calc/sum.go", 4),
            ("C:/Users/John Smith/calc/ops_test.go", 18),
        ])
    );
}

/// tsc's plain style with no `Found N errors` line: the two diagnostics are the count, and the
/// frames are in the order printed.
#[test]
fn tsc_without_a_found_line_counts_its_diagnostics() {
    let output = concat!(
        "src/ops.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.\n",
        "src/main.ts(3,10): error TS2305: Module './ops' has no exported member 'foo'.\n"
    );
    assert_eq!(
        parsed("tsc", output),
        Failures {
            counts: counts_without_passed(0, 2, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("src/ops.ts", 12), ("src/main.ts", 3)]),
            error_kind: kind("TS2322"),
        }
    );
}

/// A diagnostic about the configuration has no location: it is an error with its code and no
/// frame.
#[test]
fn tsc_error_without_a_location_is_an_error_with_no_frame() {
    let output = "error TS5083: Cannot read file '/repo/tsconfig.json'.\n";
    assert_eq!(
        parsed("tsc", output),
        Failures {
            counts: counts_without_passed(0, 1, 0),
            failed_tests: Vec::new(),
            frames: Vec::new(),
            error_kind: kind("TS5083"),
        }
    );
}

/// A Vitest suite that failed to load (the `FAIL  file [ file ]` line of Vitest 0.14.1, in the
/// vitest-dev/vscode issue 47): no test ran, so the suite is the one error, named without the
/// bracket suffix.
const VITEST_SUITE_FAILURE: &str = r"
 RUN  v0.14.1 /repo

⎯⎯⎯⎯⎯⎯ Failed Suites 1 ⎯⎯⎯⎯⎯⎯⎯

 FAIL  test/code.test.ts [ test/code.test.ts ]
Error: No test suite found in file /repo/test/code.test.ts

⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯⎯[1/1]⎯

 Test Files  1 failed (1)
      Tests  no tests
";

/// The suite is named by its file alone.
#[test]
fn vitest_suite_failure_is_named_without_its_bracket_suffix() {
    assert_eq!(
        parsed("vitest", VITEST_SUITE_FAILURE),
        Failures {
            counts: counts_without_passed(0, 1, 0),
            failed_tests: names(&["test/code.test.ts"]),
            frames: Vec::new(),
            error_kind: kind("Error"),
        }
    );
}

/// jest with a failed test in one file and a file that failed to run: the `Tests` line's counts
/// are kept (1 failed, 1 passed) and the file that ran no tests is the one error. Both are named.
const JEST_TEST_AND_SUITE_FAILED: &str = r"
 FAIL  src/a.test.js
  ● divide › divides by zero

    expect(received).toBe(expected) // Object.is equality

      at Object.<anonymous> (src/a.test.js:13:26)

 FAIL  src/b.test.js
  ● Test suite failed to run

    SyntaxError: Unexpected token ')'

Test Suites: 2 failed, 2 total
Tests:       1 failed, 1 passed, 2 total
";

/// A suite that failed to run adds an error to the counts of the tests that failed.
#[test]
fn jest_failed_tests_and_a_suite_that_failed_to_run_are_both_counted() {
    assert_eq!(
        parsed("jest", JEST_TEST_AND_SUITE_FAILED),
        Failures {
            counts: counts(1, 1, 1, 0),
            failed_tests: names(&["divide › divides by zero", "src/b.test.js"]),
            frames: frames(&[("src/a.test.js", 13)]),
            error_kind: kind("SyntaxError"),
        }
    );
}
