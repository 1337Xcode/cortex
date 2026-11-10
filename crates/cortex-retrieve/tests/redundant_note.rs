//! The note a check gets before it runs again with no source change since its earlier run: the
//! failing tests that run named, at most three and then how many more, else its exit code alone;
//! the command in backquotes only when it is plain, has no backquote and is
//! at most 80 characters; and never more than 60 tokens. Every expected text is typed out by hand.

use cortex_core::estimate_tokens;
use cortex_retrieve::{EarlierRun, MAX_REDUNDANT_NOTE_TOKENS, redundant_note};

/// A run that exited `exit_code`, its output naming `failing` of `failing_count` failing tests.
fn run(exit_code: i32, failing: &[&str], failing_count: usize) -> EarlierRun {
    EarlierRun {
        exit_code,
        failing: failing.iter().map(|name| (*name).to_owned()).collect(),
        failing_count,
    }
}

/// A failed run whose output named one failing test names it.
#[test]
fn a_failed_run_names_its_failing_test() {
    assert_eq!(
        redundant_note(
            "pytest tests/test_ops.py",
            &run(1, &["test_divide_by_zero"], 1)
        )
        .as_deref(),
        Some(
            "[cortex] `pytest tests/test_ops.py` ran earlier in this session with no source change \
             since: exit 1, failing: test_divide_by_zero.\n"
        )
    );
}

/// Of five failing tests, the first three are named and the other two counted.
#[test]
fn past_three_failing_tests_the_rest_are_counted() {
    assert_eq!(
        redundant_note("pytest", &run(1, &["test_a", "test_b", "test_c"], 5)).as_deref(),
        Some(
            "[cortex] `pytest` ran earlier in this session with no source change since: exit 1, \
             failing: test_a, test_b, test_c and 2 more.\n"
        )
    );
}

/// A failed run whose output named no test gets its exit code alone.
#[test]
fn a_failed_run_without_names_gets_its_exit_code_alone() {
    assert_eq!(
        redundant_note("cargo build", &run(101, &[], 0)).as_deref(),
        Some(
            "[cortex] `cargo build` ran earlier in this session with no source change since: exit \
             101.\n"
        )
    );
}

/// A run that passed gets `exit 0` alone: a failing name it somehow carries is not told.
#[test]
fn a_passing_run_gets_its_exit_code() {
    assert_eq!(
        redundant_note("pytest -q", &run(0, &["test_a"], 1)).as_deref(),
        Some(
            "[cortex] `pytest -q` ran earlier in this session with no source change since: exit \
             0.\n"
        )
    );
}

/// A command that is not plain, holds a backquote, or is longer than 80 characters is not shown:
/// the note says "This command". One of exactly 80 characters is shown.
#[test]
fn a_command_not_fit_to_show_is_not_shown() {
    let hidden = "[cortex] This command ran earlier in this session with no source change since: exit \
                  1, failing: test_a.\n";
    for command in [
        "pytest\necho done".to_owned(),
        "pytest \u{202e}tests".to_owned(),
        "pytest -k `cat names`".to_owned(),
        format!("pytest {}", "t".repeat(74)),
        "   ".to_owned(),
    ] {
        assert_eq!(
            redundant_note(&command, &run(1, &["test_a"], 1)).as_deref(),
            Some(hidden),
            "{command:?}"
        );
    }
    let longest = format!("pytest {}", "t".repeat(73));
    assert_eq!(longest.chars().count(), 80);
    assert_eq!(
        redundant_note(&longest, &run(1, &["test_a"], 1)),
        Some(format!(
            "[cortex] `{longest}` ran earlier in this session with no source change since: exit \
             1, failing: test_a.\n"
        ))
    );
}

/// A failing name that is not plain text is never shown: it is counted with the others.
#[test]
fn a_name_that_is_not_plain_is_counted_not_shown() {
    assert_eq!(
        redundant_note("pytest", &run(1, &["test_\u{1b}[2J", "test_b"], 2)).as_deref(),
        Some(
            "[cortex] `pytest` ran earlier in this session with no source change since: exit 1, \
             failing: test_b and 1 more.\n"
        )
    );
}

/// The note is held to 60 tokens: of three failing names of 60 characters, all three would make
/// it 278 bytes (70 tokens), so two are named and the third counted, 227 bytes (57 tokens). The
/// longest note without names, an 80-character command and the longest exit code, is 167 bytes
/// (42 tokens), and is given.
#[test]
fn the_note_is_held_to_its_cap() {
    let names: Vec<String> = ["a", "b", "c"]
        .iter()
        .map(|letter| format!("test_{}", letter.repeat(55)))
        .collect();
    let named: Vec<&str> = names.iter().map(String::as_str).collect();
    let note = redundant_note("pytest", &run(1, &named, 3));
    assert_eq!(
        note,
        Some(format!(
            "[cortex] `pytest` ran earlier in this session with no source change since: exit 1, \
             failing: {}, {} and 1 more.\n",
            names[0], names[1]
        ))
    );
    let note = note.unwrap_or_default();
    assert_eq!((note.len(), estimate_tokens(&note)), (227, 57));
    let note = redundant_note(&"x".repeat(80), &run(i32::MIN, &[], 0)).unwrap_or_default();
    assert_eq!(note.len(), 167);
    assert!(estimate_tokens(&note) <= MAX_REDUNDANT_NOTE_TOKENS);
}
