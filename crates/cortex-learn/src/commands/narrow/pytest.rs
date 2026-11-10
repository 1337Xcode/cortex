//! pytest: which arguments select tests, and how a narrow command names test files. A path or a
//! node id (`tests/test_ops.py::test_add`) selects, as do the flags that pick tests by name,
//! marker or last outcome; flags that only shape the run do not. A flag this module does not know
//! followed by a word is read as a flag and a path, so the run is uncertain.

use super::{SuiteRun, Whole};

/// The long flags that select or deselect tests, or run none.
const SELECTING: &[&str] = &[
    "--lf",
    "--last-failed",
    "--sw",
    "--stepwise",
    "--deselect",
    "--ignore",
    "--ignore-glob",
    "--pyargs",
    "--collect-only",
    "--co",
    "--doctest-modules",
    "--picked",
];

/// The flags that take their value as the next word when it is not joined with `=`.
const VALUE_FLAGS: &[&str] = &[
    "-p",
    "-c",
    "-o",
    "-W",
    "-r",
    "-n",
    "--numprocesses",
    "--dist",
    "--maxfail",
    "--durations",
    "--durations-min",
    "--tb",
    "--color",
    "--capture",
    "--import-mode",
    "--rootdir",
    "--basetemp",
    "--junitxml",
    "--junit-xml",
    "--log-level",
    "--timeout",
    "--cov-report",
    "--confcutdir",
    "--override-ini",
    "--config-file",
    "--assert",
    "--show-capture",
];

/// The letters of short flags whose value is the rest of the word, or the next word.
const SHORT_VALUE_LETTERS: &str = "pcoWrn";

/// The letters of short flags that select tests: `-k` by name, `-m` by marker.
const SHORT_SELECTING_LETTERS: &str = "km";

/// Whether the arguments after `pytest` select nothing.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        at += 1;
        if !arg.starts_with('-') || arg == "-" || arg == "--" {
            return None;
        }
        let (name, joined) = arg
            .split_once('=')
            .map_or((arg, false), |(name, _)| (name, true));
        if SELECTING.contains(&name) {
            return None;
        }
        let takes_next = if name.starts_with("--") {
            !joined && VALUE_FLAGS.contains(&name)
        } else {
            short_flags(name)?.takes_next && !joined
        };
        at += usize::from(takes_next);
    }
    Some(Whole::default())
}

/// What a bundle of short flags (`-xvs`, `-n4`, `-rA`) does.
struct Short {
    /// Whether the next word is the value of the last flag.
    takes_next: bool,
}

/// Reads the short flags of `name`; `None` when one of them selects tests.
fn short_flags(name: &str) -> Option<Short> {
    let letters = name.strip_prefix('-')?;
    for (at, letter) in letters.char_indices() {
        if SHORT_SELECTING_LETTERS.contains(letter) {
            return None;
        }
        if SHORT_VALUE_LETTERS.contains(letter) {
            let after = at + letter.len_utf8();
            return Some(Short {
                takes_next: letters.get(after..).is_none_or(str::is_empty),
            });
        }
    }
    Some(Short { takes_next: false })
}

/// The Python files among `files`: pytest takes them as paths.
pub(super) fn narrow(_run: &SuiteRun, files: &[&str]) -> Option<Vec<String>> {
    let found: Vec<String> = files
        .iter()
        .filter(|file| {
            std::path::Path::new(file)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
        })
        .map(|file| (*file).to_owned())
        .collect();
    (!found.is_empty()).then_some(found)
}
