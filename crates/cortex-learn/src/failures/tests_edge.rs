//! Edge cases of the shared parts: escape sequences, the cut of the tail, hostile text, locations
//! that are not locations, the rules for which paths are the project's, and the order frames are
//! gathered in. The expected values are counted by hand in each test.

use std::fmt::Write as _;

use super::clean::clean_tail;
use super::collect::{Collector, TraceOrder};
use super::locate::{locate_prefix, split_location};
use super::paths::is_kept_path;
use super::tests::{counts, counts_without_passed, frames, kind, names, parsed};
use super::*;

/// A single line of compact JSON of at least `min_bytes`, `{"k0":0.5,"k1":1.5,...}`: no
/// whitespace, a colon for each key and a dot in each value.
fn compact_json_line(min_bytes: usize) -> String {
    let mut line = String::from("{");
    let mut key = 0_u32;
    while line.len() < min_bytes {
        if key > 0 {
            line.push(',');
        }
        let _ = write!(line, "\"k{key}\":{key}.5");
        key += 1;
    }
    line.push('}');
    line
}

/// A 200 KiB line with a colon after every key has no location in it and is read in one pass: the
/// pytest parser's location scan returns and the failure on the next line is still found.
#[test]
fn a_long_line_of_compact_json_is_not_a_location() {
    let json = compact_json_line(200 * 1024);
    assert!(json.len() >= 200 * 1024);
    assert!(locate_prefix(&json).is_none());
    let rest = "\nFAILED tests/test_ops.py::test_a - AssertionError: x\n1 failed in 0.01s\n";
    let output = format!("{json}{rest}");
    let found = parsed("pytest", &output);
    assert_eq!(found.frames, []);
    assert_eq!(found.failed_tests, names(&["tests/test_ops.py::test_a"]));
    assert_eq!(found.counts, counts(1, 0, 0, 0));
}

/// Coloured pytest output: the colour codes around `FAILED`, inside the test id and around the
/// summary line are removed, so the failed test, its error and the counts are found.
#[test]
fn colour_codes_in_pytest_output_are_removed() {
    let output = [
        "\u{1b}[31mFAILED\u{1b}[0m tests/test_ops.py::\u{1b}[1mtest_divide\u{1b}[0m - \
         ZeroDivisionError: division by zero",
        "\u{1b}[31m\u{1b}[1m=========================== 1 failed, 7 passed in 0.12s \
         ===========================\u{1b}[0m",
    ]
    .join("\n");
    let found = parsed("pytest", &output);
    assert_eq!(
        found.failed_tests,
        names(&["tests/test_ops.py::test_divide"])
    );
    assert_eq!(found.counts, counts(1, 7, 0, 0));
    assert_eq!(found.error_kind, kind("ZeroDivisionError"));
}

/// Coloured cargo output, with 256-colour parameters (`38;5;9`): the diagnostic's code, the
/// location after the coloured arrow and the closing count are found.
#[test]
fn colour_codes_in_cargo_output_are_removed() {
    let output = [
        concat!(
            "\u{1b}[0m\u{1b}[1m\u{1b}[38;5;9merror[E0308]\u{1b}[0m",
            "\u{1b}[1m: mismatched types\u{1b}[0m"
        ),
        concat!(
            "\u{1b}[0m  \u{1b}[0m\u{1b}[1m\u{1b}[38;5;12m--> \u{1b}[0m",
            "src/main.rs:12:5\u{1b}[0m"
        ),
        concat!(
            "\u{1b}[1m\u{1b}[31merror\u{1b}[0m\u{1b}[1m:\u{1b}[0m ",
            "could not compile `calc` (bin \"calc\") due to 1 previous error"
        ),
    ]
    .join("\n");
    let found = parsed("cargo", &output);
    assert_eq!(found.counts, counts_without_passed(0, 1, 0));
    assert_eq!(found.frames, frames(&[("src/main.rs", 12)]));
    assert_eq!(found.error_kind, kind("E0308"));
}

/// An operating system command (a window title, a hyperlink) ends at BEL or at `ESC \`, and a
/// sequence that is cut off by a line break does not swallow the line.
#[test]
fn osc_sequences_are_removed_and_a_cut_off_sequence_keeps_its_line() {
    assert_eq!(clean_tail("\u{1b}]0;title\u{7}plain"), "plain");
    assert_eq!(
        clean_tail("\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\ text"),
        "link text"
    );
    assert_eq!(clean_tail("a\u{1b}[31\nb"), "a\nb");
    assert_eq!(clean_tail("a\u{1b}]0;never ended\nb"), "a\nb");
}

/// A cut that falls inside a multi-byte character moves on to the next character, and the tail
/// then starts at the next whole line. Each line is 24 two-byte letters and a newline, 49 bytes,
/// 110,000 of them and one more newline: 5,390,001 bytes. The cut is at 5,390,001 - 65,536 =
/// 5,324,465, which is 27 bytes into a line (5,324,465 = 49 * 108,662 + 27), the second byte of
/// the fourteenth letter. The tail starts at the next line, 49 * 108,663 = 5,324,487, so it is
/// 5,390,001 - 5,324,487 = 65,514 bytes.
#[test]
fn a_cut_inside_a_multi_byte_character_starts_the_tail_at_the_next_line() {
    let noise = "é".repeat(24) + "\n";
    let output = noise.repeat(110_000) + "\n";
    assert_eq!(output.len(), 5_390_001);
    assert!(!output.is_char_boundary(output.len() - MAX_OUTPUT_BYTES));
    let tail = clean_tail(&output);
    assert_eq!(MAX_OUTPUT_BYTES, 65_536);
    assert_eq!(tail.len(), 65_514);
    assert!(tail.starts_with(&"é".repeat(24)));
    assert!(tail.ends_with("\n\n"));
}

/// Control characters and the characters that hide or reorder text become spaces in a test's
/// name: a bell and a right-to-left override here.
#[test]
fn control_characters_in_a_test_name_are_replaced() {
    let output =
        "FAILED tests/test_ops.py::test_\u{7}a\u{202e}b - AssertionError: x\n1 failed in 0.01s\n";
    let found = parsed("pytest", output);
    assert_eq!(found.failed_tests, names(&["tests/test_ops.py::test_ a b"]));
}

/// An error kind is cleaned the same way and cut to [`MAX_NAME_CHARS`].
#[test]
fn an_error_kind_is_cleaned_and_cut() {
    let mut collector = Collector::default();
    collector.set_error_kind("Bad\u{7}Kind\u{200b}");
    assert_eq!(
        collector.finish().and_then(|found| found.error_kind),
        kind("Bad Kind")
    );
    let mut collector = Collector::default();
    collector.set_error_kind(&"E".repeat(300));
    let long = collector.finish().and_then(|found| found.error_kind);
    assert_eq!(long.map(|text| text.chars().count()), Some(MAX_NAME_CHARS));
}

/// Prose with colons and numbers is not a location, and drive paths and paths with spaces are.
#[test]
fn prose_is_not_a_location_and_drive_and_spaced_paths_are() {
    assert_eq!(split_location("at the time: 12:30"), None);
    assert_eq!(split_location("the time: 12"), None);
    assert_eq!(split_location("C:\\x\\y.js:1:2"), Some(("C:\\x\\y.js", 1)));
    assert_eq!(
        split_location("C:/Users/John Smith/calc/ops.go:9"),
        Some(("C:/Users/John Smith/calc/ops.go", 9))
    );
    // Through a parser: Node's stack frame without parentheses is the only place such text is
    // read as a location, and `at the time: 12:30` is no frame.
    let output = "Error: x\n    at the time: 12:30\n    at run (/repo/src/ops.js:3:1)\n";
    let found = parsed("npm", output);
    assert_eq!(found.frames, frames(&[("/repo/src/ops.js", 3)]));
}

/// Frames gathered outermost first come out innermost first, and at the cap it is the innermost
/// ones that stay: thirty frames, `f0` outermost, leave `f29` down to `f10`.
#[test]
fn an_outermost_first_trace_keeps_its_innermost_frames_at_the_cap() {
    let mut collector = Collector::default();
    collector.begin_trace(TraceOrder::OutermostFirst);
    for number in 0..30_u32 {
        collector.add_frame(&format!("pkg/f{number}.py"), number + 1);
    }
    collector.end_trace();
    let found = collector.finish().unwrap_or_default();
    assert_eq!(found.frames.len(), MAX_FRAMES);
    assert_eq!(found.frames.first(), frames(&[("pkg/f29.py", 30)]).first());
    assert_eq!(found.frames.last(), frames(&[("pkg/f10.py", 11)]).first());
}

/// Failures come in output order, each trace in its own order, and a frame seen again is kept
/// once: `a`, `b` innermost first, then `c`, `d` outermost first (so `d`, `c`), then `a` again.
#[test]
fn traces_of_both_orders_follow_one_another_without_repeats() {
    let mut collector = Collector::default();
    collector.begin_trace(TraceOrder::InnermostFirst);
    collector.add_frame("a.py", 1);
    collector.add_frame("b.py", 2);
    collector.begin_trace(TraceOrder::OutermostFirst);
    collector.add_frame("c.py", 3);
    collector.add_frame("d.py", 4);
    collector.add_frame("a.py", 1);
    collector.end_trace();
    let found = collector.finish().unwrap_or_default();
    assert_eq!(
        found.frames,
        frames(&[("a.py", 1), ("b.py", 2), ("d.py", 4), ("c.py", 3)])
    );
}

/// The interpreter's library, the cargo registry and the Rust standard library are not the
/// project's; a project's own folders that look like them are.
#[test]
fn library_paths_are_dropped_by_rule_and_lookalikes_are_kept() {
    let dropped = [
        "/usr/lib/python3.12/importlib/__init__.py",
        "/usr/local/lib/python2.7/os.py",
        "../../anaconda3/envs/app/lib/python3.10/importlib/__init__.py",
        "C:/Users/x/AppData/Local/Programs/Python/Python312/Lib/json/__init__.py",
        "C:/Python27/Lib/os.py",
        "C:/Users/x/miniconda3/Lib/json/decoder.py",
        "/home/u/.cargo/registry/src/index.crates.io-6f17d22bba15001f/serde-1.0.0/src/de.rs",
        "/home/u/.cargo/git/checkouts/dep-1a2b3c/abc/src/lib.rs",
        "/rustc/abc123/library/core/src/option.rs",
        "/rustc/abc123/library/std/src/panicking.rs",
        "library/alloc/src/vec/mod.rs",
        "library/std/src/rt.rs",
    ];
    for path in dropped {
        assert!(!is_kept_path(path), "{path} should be dropped");
    }
    let kept = [
        "pkg/library/core/x.py",
        "/work/Lib/helpers.py",
        "C:/work/Lib/x.py",
        "/work/lib/python/x.py",
        "src/library.rs",
        "calc/ops.py",
    ];
    for path in kept {
        assert!(is_kept_path(path), "{path} should be kept");
    }
}
