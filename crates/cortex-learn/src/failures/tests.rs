//! The parsers on realistic output typed in whole, each with the findings counted by hand from
//! the text: the counts as the summary line prints them, the names and frames in the order they
//! appear, and the first error's kind. The caps, output that shows no failure and output far
//! bigger than the cap are covered at the end. Each failure's frames are listed innermost first.
//! `tests_formats` and `tests_edge` hold the newer formats and the edge cases and use this
//! file's helpers.

use super::clean::clean_tail;
use super::*;

/// The findings of `runner`'s parser on `output`, which must show a failure.
pub(super) fn parsed(runner: &str, output: &str) -> Failures {
    let found = parse_failures(runner, output);
    assert!(found.is_some(), "{runner}: no failure found");
    found.unwrap_or_default()
}

/// `names` as owned strings.
pub(super) fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| (*name).to_owned()).collect()
}

/// Frames from `(path, line)` pairs.
pub(super) fn frames(list: &[(&str, u32)]) -> Vec<Frame> {
    list.iter()
        .map(|(path, line)| Frame {
            path: (*path).to_owned(),
            line: *line,
        })
        .collect()
}

/// Counts in the order failed, passed, errors, skipped, with the passing count printed.
#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with the optional counts a parser returns"
)]
pub(super) fn counts(failed: u32, passed: u32, errors: u32, skipped: u32) -> Option<Counts> {
    Some(Counts {
        failed,
        passed: Some(passed),
        errors,
        skipped,
    })
}

/// Counts in the order failed, errors, skipped, for a runner that printed no passing count.
#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with the optional counts a parser returns"
)]
pub(super) fn counts_without_passed(failed: u32, errors: u32, skipped: u32) -> Option<Counts> {
    Some(Counts {
        failed,
        passed: None,
        errors,
        skipped,
    })
}

/// An error kind.
#[allow(
    clippy::unnecessary_wraps,
    reason = "compared with the optional kind a parser returns"
)]
pub(super) fn kind(text: &str) -> Option<String> {
    Some(text.to_owned())
}

/// Lines joined with newlines, for output that holds tabs.
pub(super) fn joined(lines: &[&str]) -> String {
    lines.join("\n")
}

/// pytest's default long style: one failed test of ten, 7 passed, 2 skipped. The location lines
/// `tests/test_ops.py:14:` and `calc/ops.py:9: ZeroDivisionError` are the two frames (the test's
/// first in the output, so `calc/ops.py` is the innermost and comes first in the result), the `E`
/// line and the summary entry name the error, and the warnings summary's `path:line` line is no
/// frame.
const PYTEST_LONG: &str = r"
============================= test session starts ==============================
platform linux -- Python 3.12.1, pytest-8.1.1, pluggy-1.4.0
collected 10 items

tests/test_ops.py ..F.                                                   [ 40%]
tests/test_more.py ......                                                [100%]

=================================== FAILURES ===================================
________________________________ test_divide ___________________________________

    def test_divide():
>       assert divide(1, 0) == 0

tests/test_ops.py:14:
_ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _ _

a = 1, b = 0

    def divide(a, b):
>       return a / b
E       ZeroDivisionError: division by zero

calc/ops.py:9: ZeroDivisionError
=============================== warnings summary ===============================
tests/test_ops.py:3
  /repo/tests/test_ops.py:3: DeprecationWarning: the imp module is deprecated
    import imp

-- Docs: https://docs.pytest.org/en/stable/how-to/capture-warnings.html
=========================== short test summary info ============================
FAILED tests/test_ops.py::test_divide - ZeroDivisionError: division by zero
============== 1 failed, 7 passed, 2 skipped, 1 warning in 0.12s ===============
";

/// Pytest long style gives counts test frames and error.
#[test]
fn pytest_long_style_gives_counts_test_frames_and_error() {
    assert_eq!(
        parsed("pytest", PYTEST_LONG),
        Failures {
            counts: counts(1, 7, 0, 2),
            failed_tests: names(&["tests/test_ops.py::test_divide"]),
            frames: frames(&[("calc/ops.py", 9), ("tests/test_ops.py", 14)]),
            error_kind: kind("ZeroDivisionError"),
        }
    );
}

/// pytest's short traceback style and its quiet summary line, which has no rule around it.
const PYTEST_SHORT: &str = r"
F
=================================== FAILURES ===================================
________________________________ test_divide ___________________________________
tests/test_ops.py:14: in test_divide
    assert divide(1, 0) == 0
calc/ops.py:9: in divide
    return a / b
E   ZeroDivisionError: division by zero
FAILED tests/test_ops.py::test_divide - ZeroDivisionError: division by zero
1 failed in 0.05s
";

/// Pytest short style and quiet summary.
#[test]
fn pytest_short_style_and_quiet_summary() {
    assert_eq!(
        parsed("pytest", PYTEST_SHORT),
        Failures {
            counts: counts(1, 0, 0, 0),
            failed_tests: names(&["tests/test_ops.py::test_divide"]),
            frames: frames(&[("calc/ops.py", 9), ("tests/test_ops.py", 14)]),
            error_kind: kind("ZeroDivisionError"),
        }
    );
}

/// A verbose run names the failed test in its progress line and again in the short summary.
const PYTEST_VERBOSE: &str = r"
tests/test_ops.py::test_add PASSED                                       [ 50%]
tests/test_ops.py::test_divide[1-0] FAILED                               [100%]

=========================== short test summary info ============================
FAILED tests/test_ops.py::test_divide[1-0] - assert 1 == 0
========================= 1 failed, 1 passed in 0.03s ==========================
";

/// Pytest verbose run names a failed test once.
#[test]
fn pytest_verbose_run_names_a_failed_test_once() {
    assert_eq!(
        parsed("pytest", PYTEST_VERBOSE),
        Failures {
            counts: counts(1, 1, 0, 0),
            failed_tests: names(&["tests/test_ops.py::test_divide[1-0]"]),
            frames: Vec::new(),
            error_kind: None,
        }
    );
}

/// A test module that cannot be imported, in the shape pytest 7 prints (pytest-dev/pytest issue
/// 11705 shows it from pytest 7.3.1; the same block is in the VS Code Python issue 15062 from
/// pytest 6.2.1): the section's `Traceback:` block names the interpreter's `importlib` frame and
/// then the test module's. The `importlib` frame is dropped (`/lib/python3.`), the test module's
/// is the one frame, the error count is 1, no test failed and the error's name is the `E` line's.
/// The captured log line that starts with `ERROR` is not a test.
const PYTEST_COLLECTION_ERROR: &str = r"
==================================== ERRORS ====================================
_______________________ ERROR collecting tests/test_ops.py _______________________
ImportError while importing test module '/repo/tests/test_ops.py'.
Hint: make sure your test modules/packages have valid Python names.
Traceback:
/usr/lib/python3.12/importlib/__init__.py:90: in import_module
    return _bootstrap._gcd_import(name[level:], package, level)
tests/test_ops.py:1: in <module>
    from calc import divide
E   ModuleNotFoundError: No module named 'calc'
ERROR    root:setup.py:3 could not read config
=========================== short test summary info ============================
ERROR tests/test_ops.py
!!!!!!!!!!!!!!!!!!!! Interrupted: 1 error during collection !!!!!!!!!!!!!!!!!!!!
=============================== 1 error in 0.08s ===============================
";

/// Pytest collection error counts an error and keeps the test module's frame only.
#[test]
fn pytest_collection_error_counts_an_error_and_drops_the_interpreters_frame() {
    assert_eq!(
        parsed("pytest", PYTEST_COLLECTION_ERROR),
        Failures {
            counts: counts(0, 0, 1, 0),
            failed_tests: names(&["tests/test_ops.py"]),
            frames: frames(&[("tests/test_ops.py", 1)]),
            error_kind: kind("ModuleNotFoundError"),
        }
    );
}

/// A chained exception: the `<frozen>` and `site-packages` frames are dropped, the absolute path
/// is kept as printed, and the two tracebacks are one failure. Python prints the frames outermost
/// first, `/repo/main.py:20` then `calc/ops.py:9` then (second traceback) `calc/ops.py:12`; the
/// result has them reversed, the last traceback's innermost first. `ValueError` escaped, so it is
/// the kind, not the `KeyError` it was raised while handling.
const PYTHON_TRACEBACK: &str = r#"
Traceback (most recent call last):
  File "<frozen runpy>", line 198, in _run_module_as_main
  File "/repo/main.py", line 20, in <module>
    run()
  File "/usr/lib/python3.12/site-packages/x/y.py", line 3, in f
    g()
  File "calc/ops.py", line 9, in divide
    return a / b
KeyError: 'x'

During handling of the above exception, another exception occurred:

Traceback (most recent call last):
  File "calc/ops.py", line 12, in divide
    raise ValueError("bad")
ValueError: bad
"#;

/// A chained traceback gives its frames innermost first and the escaped error.
#[test]
fn a_chained_traceback_gives_its_frames_innermost_first_and_the_escaped_error() {
    assert_eq!(
        parsed("unittest", PYTHON_TRACEBACK),
        Failures {
            counts: None,
            failed_tests: Vec::new(),
            frames: frames(&[
                ("calc/ops.py", 12),
                ("calc/ops.py", 9),
                ("/repo/main.py", 20)
            ]),
            error_kind: kind("ValueError"),
        }
    );
}

/// A Windows traceback: backslashes become slashes.
const PYTHON_WINDOWS_TRACEBACK: &str = r#"
Traceback (most recent call last):
  File "C:\work\calc\ops.py", line 9, in divide
    return a / b
ZeroDivisionError: division by zero
"#;

/// Python traceback paths use forward slashes.
#[test]
fn python_traceback_paths_use_forward_slashes() {
    assert_eq!(
        parsed("unittest", PYTHON_WINDOWS_TRACEBACK).frames,
        frames(&[("C:/work/calc/ops.py", 9)])
    );
}

/// unittest: two failed tests, 8 ran, 1 failure, 1 error, 2 skipped, so 4 passed.
const UNITTEST: &str = r#"
F.E
======================================================================
FAIL: test_divide (tests.test_ops.TestOps.test_divide)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "/repo/tests/test_ops.py", line 14, in test_divide
    self.assertEqual(divide(1, 2), 1)
AssertionError: 0 != 1

======================================================================
ERROR: test_parse (tests.test_ops.TestOps.test_parse)
----------------------------------------------------------------------
Traceback (most recent call last):
  File "/repo/tests/test_ops.py", line 20, in test_parse
    parse("x")
  File "calc/ops.py", line 4, in parse
    return int(text)
ValueError: invalid literal for int() with base 10: 'x'

----------------------------------------------------------------------
Ran 8 tests in 0.002s

FAILED (failures=1, errors=1, skipped=2)
"#;

/// Unittest gives counts names frames and the first error.
#[test]
fn unittest_gives_counts_names_frames_and_the_first_error() {
    assert_eq!(
        parsed("unittest", UNITTEST),
        Failures {
            counts: counts(1, 4, 1, 2),
            failed_tests: names(&[
                "test_divide (tests.test_ops.TestOps.test_divide)",
                "test_parse (tests.test_ops.TestOps.test_parse)",
            ]),
            // The first failure's one frame, then the second's two, innermost first.
            frames: frames(&[
                ("/repo/tests/test_ops.py", 14),
                ("calc/ops.py", 4),
                ("/repo/tests/test_ops.py", 20),
            ]),
            error_kind: kind("AssertionError"),
        }
    );
}

/// cargo test, current panic form: 2 passed, 1 failed, 1 ignored; the name comes from the
/// `... FAILED` line, the header and the list, and counts once.
const CARGO_TEST: &str = r"
   Compiling calc v0.1.0 (/repo)
    Finished `test` profile [unoptimized + debuginfo] target(s) in 0.52s
     Running unittests src/lib.rs (target/debug/deps/calc-0123456789abcdef)

running 4 tests
test tests::add ... ok
test tests::divide ... FAILED
test tests::sub ... ok
test tests::skipped ... ignored

failures:

---- tests::divide stdout ----

thread 'tests::divide' panicked at src/ops.rs:14:5:
attempt to divide by zero
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace


failures:
    tests::divide

test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.00s

error: test failed, to rerun pass `--lib`
";

/// Cargo test gives counts name and panic frame.
#[test]
fn cargo_test_gives_counts_name_and_panic_frame() {
    assert_eq!(
        parsed("cargo", CARGO_TEST),
        Failures {
            counts: counts(1, 2, 0, 1),
            failed_tests: names(&["tests::divide"]),
            frames: frames(&[("src/ops.rs", 14)]),
            error_kind: None,
        }
    );
}

/// cargo test with `--no-fail-fast`, the older panic form and two test binaries: the counts of
/// both are added (1 + 4 passed, 1 failed, 0 + 1 ignored).
const CARGO_TEST_TWO_BINARIES: &str = r"
running 2 tests
test ops::add ... ok
test ops::sub ... FAILED

failures:

---- ops::sub stdout ----
thread 'ops::sub' panicked at 'index out of bounds', src/ops.rs:20:9
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace

failures:
    ops::sub

test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

     Running tests/api.rs (target/debug/deps/api-0123456789abcdef)

running 5 tests
test a ... ok
test b ... ok
test c ... ok
test d ... ok
test e ... ignored

test result: ok. 4 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.01s
";

/// Cargo test adds binaries and reads the older panic form.
#[test]
fn cargo_test_adds_binaries_and_reads_the_older_panic_form() {
    assert_eq!(
        parsed("cargo", CARGO_TEST_TWO_BINARIES),
        Failures {
            counts: counts(1, 5, 0, 1),
            failed_tests: names(&["ops::sub"]),
            frames: frames(&[("src/ops.rs", 20)]),
            error_kind: None,
        }
    );
}

/// A panic inside the standard library names a file under `/rustc/`; the frame is dropped and the
/// test is still named.
const CARGO_TEST_STD_PANIC: &str = r"
test tests::t ... FAILED

failures:

---- tests::t stdout ----
thread 'tests::t' panicked at /rustc/abc123/library/core/src/option.rs:1:1:
msg

failures:
    tests::t

test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
";

/// Cargo test drops standard library frames.
#[test]
fn cargo_test_drops_standard_library_frames() {
    assert_eq!(
        parsed("cargo", CARGO_TEST_STD_PANIC),
        Failures {
            counts: counts(1, 0, 0, 0),
            failed_tests: names(&["tests::t"]),
            frames: Vec::new(),
            error_kind: None,
        }
    );
}

/// A build that does not compile, as cargo prints it (the closing line has the shape of the
/// output in cargo issue 13888, May 2024: the crate's kind in double quotes, and no `aborting
/// due to` line): two errors with their locations, and a warning whose location is not one. The
/// first error's code is the kind and the closing line gives the count.
const RUSTC: &str = r#"
   Compiling calc v0.1.0 (/repo)
error[E0308]: mismatched types
  --> src/main.rs:12:5
   |
12 |     let x: u32 = 1.5;
   |                  ^^^ expected `u32`, found floating-point number

warning: unused variable: `y`
 --> src/main.rs:20:9
  |
20|     let y = 1;

error[E0425]: cannot find value `z` in this scope
  --> src/lib.rs:7:13
   |
7  |     z
   |     ^ not found in this scope

Some errors have detailed explanations: E0308, E0425.
For more information about an error, try `rustc --explain E0308`.
error: could not compile `calc` (bin "calc") due to 2 previous errors; 1 warning emitted
"#;

/// Rustc errors give the code the locations and the count.
#[test]
fn rustc_errors_give_the_code_the_locations_and_the_count() {
    assert_eq!(
        parsed("cargo", RUSTC),
        Failures {
            counts: counts_without_passed(0, 2, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("src/main.rs", 12), ("src/lib.rs", 7)]),
            error_kind: kind("E0308"),
        }
    );
}

/// jest: one failed test of eight. The frame in `node_modules` is dropped.
const JEST: &str = r"
 FAIL  src/ops.test.js
  ● divide › divides by zero

    expect(received).toBe(expected) // Object.is equality

    Expected: 1
    Received: 2

      12 |   it('divides by zero', () => {
    > 13 |     expect(divide(1, 2)).toBe(1);
         |                          ^
      14 |   });

      at Object.<anonymous> (src/ops.test.js:13:26)
      at Object.<anonymous> (node_modules/jest-circus/build/run.js:5:1)

Test Suites: 1 failed, 1 total
Tests:       1 failed, 7 passed, 8 total
Snapshots:   0 total
Time:        1.2 s
";

/// Jest gives counts the header name and the test frame.
#[test]
fn jest_gives_counts_the_header_name_and_the_test_frame() {
    assert_eq!(
        parsed("jest", JEST),
        Failures {
            counts: counts(1, 7, 0, 0),
            failed_tests: names(&["divide › divides by zero"]),
            frames: frames(&[("src/ops.test.js", 13)]),
            error_kind: None,
        }
    );
}

/// jest run through npm: the error type comes from the line under the header, and the two
/// frames are the code and its test.
const JEST_THROWS: &str = r"
> calc@1.0.0 test
> jest

 FAIL  src/ops.test.js
  ● divide › throws

    TypeError: Cannot read properties of undefined (reading 'x')

      at divide (src/ops.js:9:11)
      at Object.<anonymous> (src/ops.test.js:5:3)

Tests:       1 failed, 1 passed, 2 total
";

/// Jest through npm names the error type.
#[test]
fn jest_through_npm_names_the_error_type() {
    assert_eq!(
        parsed("npm", JEST_THROWS),
        Failures {
            counts: counts(1, 1, 0, 0),
            failed_tests: names(&["divide › throws"]),
            frames: frames(&[("src/ops.js", 9), ("src/ops.test.js", 5)]),
            error_kind: kind("TypeError"),
        }
    );
}

/// A jest file that cannot run has no failed test: its failed suite counts as the error, and the
/// file is named.
const JEST_SUITE_FAILED: &str = r"
 FAIL  src/broken.test.js
  ● Test suite failed to run

    SyntaxError: Unexpected token ')'

      at Runtime.createScriptFromCode (node_modules/jest-runtime/build/index.js:1:1)

Test Suites: 1 failed, 1 total
Tests:       0 total
";

/// Jest suite that failed to run counts as an error.
#[test]
fn jest_suite_that_failed_to_run_counts_as_an_error() {
    assert_eq!(
        parsed("jest", JEST_SUITE_FAILED),
        Failures {
            counts: counts(0, 0, 1, 0),
            failed_tests: names(&["src/broken.test.js"]),
            frames: Vec::new(),
            error_kind: kind("SyntaxError"),
        }
    );
}

/// jest's verbose list marks a failed test with a cross and its duration.
const JEST_VERBOSE: &str = r"
  divide
    ✓ divides evenly (2 ms)
    ✕ divides by zero (3 ms)

Tests:       1 failed, 1 passed, 2 total
";

/// Jest verbose marks name a failed test without its duration.
#[test]
fn jest_verbose_marks_name_a_failed_test_without_its_duration() {
    assert_eq!(
        parsed("jest", JEST_VERBOSE),
        Failures {
            counts: counts(1, 1, 0, 0),
            failed_tests: names(&["divides by zero"]),
            frames: Vec::new(),
            error_kind: None,
        }
    );
}

/// vitest: its summary uses bars, its failing test comes after `FAIL` with the suite names, and
/// its frame starts with an arrow. The cross line names the same test less fully and is left out.
const VITEST: &str = r"
 ❯ src/ops.test.ts (8 tests | 1 failed) 5ms
   × divide > divides by zero 2ms
     → expected 2 to be 1 // Object.is equality

⎯⎯⎯⎯⎯⎯⎯ Failed Tests 1 ⎯⎯⎯⎯⎯⎯⎯

 FAIL  src/ops.test.ts > divide > divides by zero
AssertionError: expected 2 to be 1 // Object.is equality

- Expected
+ Received

- 1
+ 2

 ❯ src/ops.test.ts:14:26

 Test Files  1 failed (1)
      Tests  1 failed | 7 passed (8)
";

/// Vitest gives counts the full test name and the arrow frame.
#[test]
fn vitest_gives_counts_the_full_test_name_and_the_arrow_frame() {
    assert_eq!(
        parsed("vitest", VITEST),
        Failures {
            counts: counts(1, 7, 0, 0),
            failed_tests: names(&["src/ops.test.ts > divide > divides by zero"]),
            frames: frames(&[("src/ops.test.ts", 14)]),
            error_kind: kind("AssertionError"),
        }
    );
}

/// An uncaught error in Node: frames follow the error line; Node's own, `node_modules`, the
/// frame with no file and the `file://` address are handled, in that order.
const NODE_UNCAUGHT: &str = r#"
/repo/src/ops.js:14
    throw new Error("division by zero");
    ^

Error: division by zero
    at divide (/repo/src/ops.js:14:11)
    at Object.<anonymous> (/repo/src/main.js:3:1)
    at Module._compile (node:internal/modules/cjs/loader:1256:14)
    at /repo/node_modules/x/index.js:10:5
    at async Promise.all (index 0)
    at file:///repo/src/esm.mjs:7:3

Node.js v20.11.0
"#;

/// Node stack keeps project frames after the error.
#[test]
fn node_stack_keeps_project_frames_after_the_error() {
    assert_eq!(
        parsed("npm", NODE_UNCAUGHT),
        Failures {
            counts: None,
            failed_tests: Vec::new(),
            frames: frames(&[
                ("/repo/src/ops.js", 14),
                ("/repo/src/main.js", 3),
                ("/repo/src/esm.mjs", 7),
            ]),
            error_kind: kind("Error"),
        }
    );
}

/// Go test counts the result lines and names the failed test.
#[test]
fn go_test_counts_the_result_lines_and_names_the_failed_test() {
    let output = joined(&[
        "=== RUN   TestAdd",
        "--- PASS: TestAdd (0.00s)",
        "=== RUN   TestDivide",
        "    ops_test.go:12: expected 1, got 2",
        "--- FAIL: TestDivide (0.00s)",
        "=== RUN   TestSkip",
        "--- SKIP: TestSkip (0.00s)",
        "FAIL",
        "FAIL\tgithub.com/x/calc\t0.012s",
        "ok  \tgithub.com/x/other\t0.003s",
    ]);
    assert_eq!(
        parsed("go", &output),
        Failures {
            counts: counts(1, 1, 0, 1),
            failed_tests: names(&["TestDivide"]),
            frames: frames(&[("ops_test.go", 12)]),
            error_kind: None,
        }
    );
}

/// A panic after the test's own `--- FAIL` line is that one failure, not an error besides it; no
/// `-v` output, so no passing count; the goroutine's frames are the project's two (the toolchain's
/// two are dropped), innermost first as printed.
#[test]
fn a_go_panic_in_a_failed_test_is_one_failure_with_the_projects_frames_only() {
    let output = joined(&[
        "--- FAIL: TestIndex (0.00s)",
        "panic: runtime error: index out of range [5] with length 3 [recovered]",
        "\tpanic: runtime error: index out of range [5] with length 3",
        "",
        "goroutine 7 [running]:",
        "testing.tRunner.func1.2({0x5a1e40, 0xc000012345})",
        "\t/usr/local/go/src/testing/testing.go:1631 +0x24a",
        "panic({0x5a1e40, 0xc000012345})",
        "\t/usr/local/go/src/runtime/panic.go:884 +0x213",
        "github.com/x/calc.Index(...)",
        "\t/work/calc/ops.go:9",
        "github.com/x/calc.TestIndex(0xc000102000?)",
        "\t/work/calc/ops_test.go:18 +0x1d",
        "FAIL\tgithub.com/x/calc\t0.015s",
    ]);
    assert_eq!(
        parsed("go", &output),
        Failures {
            counts: counts_without_passed(1, 0, 0),
            failed_tests: names(&["TestIndex"]),
            frames: frames(&[("/work/calc/ops.go", 9), ("/work/calc/ops_test.go", 18)]),
            error_kind: kind("panic"),
        }
    );
}

/// Go build failure gives compiler locations and an error.
#[test]
fn go_build_failure_gives_compiler_locations_and_an_error() {
    let output = joined(&[
        "# github.com/x/calc",
        "./ops.go:9:2: undefined: foo",
        "./ops.go:12:5: declared and not used: x",
        "FAIL\tgithub.com/x/calc [build failed]",
    ]);
    assert_eq!(
        parsed("go", &output),
        Failures {
            counts: counts_without_passed(0, 1, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("./ops.go", 9), ("./ops.go", 12)]),
            error_kind: kind("build failed"),
        }
    );
}

/// tsc's plain style: the count comes from the `Found` line.
const TSC: &str = r#"
src/ops.ts(12,5): error TS2322: Type 'string' is not assignable to type 'number'.
src/main.ts(3,10): error TS2305: Module '"./ops"' has no exported member 'foo'.

Found 2 errors in 2 files.

Errors  Files
     1  src/ops.ts:12
     1  src/main.ts:3
"#;

/// Tsc plain style gives frames the first code and the count.
#[test]
fn tsc_plain_style_gives_frames_the_first_code_and_the_count() {
    assert_eq!(
        parsed("tsc", TSC),
        Failures {
            counts: counts_without_passed(0, 2, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("src/ops.ts", 12), ("src/main.ts", 3)]),
            error_kind: kind("TS2322"),
        }
    );
}

/// tsc's pretty style, run through pnpm, whose output jest's parser reads first and finds nothing.
const TSC_PRETTY: &str = r#"
src/ops.ts:12:5 - error TS2322: Type 'string' is not assignable to type 'number'.

12     const x: number = "a";
       ~

Found 1 error in src/ops.ts:12
"#;

/// Tsc pretty style through a package manager.
#[test]
fn tsc_pretty_style_through_a_package_manager() {
    assert_eq!(
        parsed("pnpm", TSC_PRETTY),
        Failures {
            counts: counts_without_passed(0, 1, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("src/ops.ts", 12)]),
            error_kind: kind("TS2322"),
        }
    );
}

/// Maven gives totals the test the project frame and the error type.
#[test]
fn maven_gives_totals_the_test_the_project_frame_and_the_error_type() {
    let output = joined(&[
        "[INFO] Running com.x.OpsTest",
        concat!(
            "[ERROR] Tests run: 3, Failures: 1, Errors: 0, Skipped: 0, ",
            "Time elapsed: 0.05 s <<< FAILURE! - in com.x.OpsTest"
        ),
        "[ERROR] divide  Time elapsed: 0.01 s  <<< FAILURE!",
        "java.lang.AssertionError: expected:<1> but was:<2>",
        "\tat org.junit.Assert.fail(Assert.java:89)",
        "\tat org.junit.Assert.assertEquals(Assert.java:645)",
        "\tat com.x.OpsTest.divide(OpsTest.java:14)",
        "\tat java.base/jdk.internal.reflect.NativeMethodAccessorImpl.invoke0(Native Method)",
        "\tat java.base/java.lang.reflect.Method.invoke(Method.java:566)",
        "",
        "[INFO]",
        "[INFO] Results:",
        "[ERROR] Failures:",
        "[ERROR]   OpsTest.divide:14 expected:<1> but was:<2>",
        "[INFO]",
        "[ERROR] Tests run: 8, Failures: 1, Errors: 0, Skipped: 0",
    ]);
    assert_eq!(
        parsed("mvn", &output),
        Failures {
            counts: counts(1, 7, 0, 0),
            failed_tests: names(&["divide"]),
            frames: frames(&[("com/x/OpsTest.java", 14)]),
            error_kind: kind("java.lang.AssertionError"),
        }
    );
}

/// Maven compile error gives its location and count.
#[test]
fn maven_compile_error_gives_its_location_and_count() {
    let output = joined(&[
        "[ERROR] COMPILATION ERROR :",
        "[ERROR] /work/calc/src/main/java/com/x/Ops.java:[12,5] cannot find symbol",
        "[INFO] 1 error",
    ]);
    assert_eq!(
        parsed("mvn", &output),
        Failures {
            counts: counts_without_passed(0, 1, 0),
            failed_tests: Vec::new(),
            frames: frames(&[("/work/calc/src/main/java/com/x/Ops.java", 12)]),
            error_kind: None,
        }
    );
}

/// Gradle gives the test the trace frame and totals.
#[test]
fn gradle_gives_the_test_the_trace_frame_and_totals() {
    let output = joined(&[
        "> Task :test FAILED",
        "",
        "OpsTest > divide() FAILED",
        "    java.lang.AssertionError at OpsTest.java:14",
        "",
        "8 tests completed, 1 failed",
        "",
        "FAILURE: Build failed with an exception.",
    ]);
    assert_eq!(
        parsed("gradle", &output),
        Failures {
            counts: counts(1, 7, 0, 0),
            failed_tests: names(&["OpsTest > divide()"]),
            frames: frames(&[("OpsTest.java", 14)]),
            error_kind: kind("java.lang.AssertionError"),
        }
    );
}

/// Output that shows no failure gives none.
#[test]
fn output_that_shows_no_failure_gives_none() {
    assert_eq!(
        parse_failures("pytest", "============ 8 passed in 0.10s ============\n"),
        None
    );
    let passing_cargo = concat!(
        "running 1 test\ntest a ... ok\n\n",
        "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; ",
        "finished in 0.00s\n"
    );
    assert_eq!(parse_failures("cargo", passing_cargo), None);
    assert_eq!(
        parse_failures("jest", "Tests:       8 passed, 8 total\n"),
        None
    );
    assert_eq!(
        parse_failures("go", "ok  \tgithub.com/x/calc\t0.003s\n"),
        None
    );
    assert_eq!(
        parse_failures(
            "mvn",
            "[INFO] Tests run: 8, Failures: 0, Errors: 0, Skipped: 0\n"
        ),
        None
    );
    assert_eq!(parse_failures("pytest", ""), None);
}

/// A runner with no parser gives none.
#[test]
fn a_runner_with_no_parser_gives_none() {
    assert_eq!(parse_failures("make", PYTEST_LONG), None);
    assert_eq!(parse_failures("dotnet", PYTEST_LONG), None);
    // Not rows of the registry: bun's own format is not read, and python and node are not
    // runner names the command registry gives.
    assert_eq!(parse_failures("bun", JEST), None);
    assert_eq!(parse_failures("python", PYTHON_TRACEBACK), None);
    assert_eq!(parse_failures("node", NODE_UNCAUGHT), None);
    assert_eq!(parse_failures("", PYTEST_LONG), None);
}

/// Names and frames stop at their caps and counts do not.
#[test]
fn names_and_frames_stop_at_their_caps_and_counts_do_not() {
    let failed: Vec<String> = (0..30)
        .map(|number| format!("test tests::t{number} ... FAILED"))
        .collect();
    let panics: Vec<String> = (0..30)
        .map(|number| {
            let line = number + 1;
            format!("thread 'tests::t{number}' panicked at src/f{number}.rs:{line}:1:")
        })
        .collect();
    let output = format!(
        concat!(
            "{}\n\nfailures:\n\n{}\n\n",
            "test result: FAILED. 0 passed; 30 failed; 0 ignored; 0 measured; ",
            "0 filtered out; finished in 0.00s\n"
        ),
        failed.join("\n"),
        panics.join("\n"),
    );
    let found = parsed("cargo", &output);
    let expected_names: Vec<String> = (0..20).map(|number| format!("tests::t{number}")).collect();
    let expected_frames: Vec<Frame> = (0..20)
        .map(|number| Frame {
            path: format!("src/f{number}.rs"),
            line: number + 1,
        })
        .collect();
    assert_eq!(found.failed_tests, expected_names);
    assert_eq!(found.frames, expected_frames);
    assert_eq!(found.counts, counts(30, 0, 0, 0));
}

/// A test name is cut to its cap.
#[test]
fn a_test_name_is_cut_to_its_cap() {
    let long = "a".repeat(300);
    let output =
        format!("FAILED tests/test_ops.py::test_{long} - AssertionError: x\n1 failed in 0.01s\n");
    let found = parsed("pytest", &output);
    assert_eq!(found.failed_tests.len(), 1);
    let name = found.failed_tests.first().map_or("", String::as_str);
    assert_eq!(name.chars().count(), MAX_NAME_CHARS);
    assert!(name.starts_with("tests/test_ops.py::test_aaa"), "{name}");
}

/// Five megabytes of output is read from its tail within the caps.
#[test]
fn five_megabytes_of_output_is_read_from_its_tail_within_the_caps() {
    // 49 bytes a line (24 two-byte letters and the newline), so that the cut falls inside a letter
    // or a line depending on the length.
    let noise = "é".repeat(24) + "\n";
    let mut output = String::from("test early ... FAILED\n");
    output.push_str(&noise.repeat(110_000));
    output.push_str(concat!(
        "test late ... FAILED\n\n",
        "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; ",
        "0 filtered out; finished in 0.00s\n"
    ));
    assert!(output.len() > 5 * 1024 * 1024);
    let found = parsed("cargo", &output);
    assert_eq!(found.failed_tests, names(&["late"]));
    assert_eq!(found.counts, counts(1, 0, 0, 0));
    assert!(found.frames.len() <= MAX_FRAMES);
}

/// The tail starts at a whole line and is at most the cap.
#[test]
fn the_tail_starts_at_a_whole_line_and_is_at_most_the_cap() {
    let noise = "é".repeat(24) + "\n";
    let output = noise.repeat(110_000);
    let tail = clean_tail(&output);
    assert!(tail.len() <= MAX_OUTPUT_BYTES);
    assert!(tail.len() >= MAX_OUTPUT_BYTES - 100);
    assert!(tail.starts_with(&"é".repeat(24)));
    assert_eq!(clean_tail("short"), "short");
}
