//! pytest: which arguments select tests, and how a narrow command names test files. A path or a
//! node id (`tests/test_ops.py::test_add`) selects, as do the flags that pick tests by name,
//! marker or last outcome. Only flags listed here as leaving the tests alone keep a run whole;
//! any other, a short letter included, makes it uncertain (`--sw-skip` selects, `--fixtures` and
//! `-h` run no test). The listed flags are repeated in the narrow command, except those that
//! report on the run or check its coverage: a narrow run's coverage falls short of a threshold the
//! whole suite meets, and its report would replace the suite's.

use super::{Narrowed, SuiteRun, Whole};

/// The long flags that leave the tests alone and take no value.
const PLAIN_FLAGS: &[&str] = &[
    "--exitfirst",
    "--verbose",
    "--quiet",
    "--no-header",
    "--no-summary",
    "--showlocals",
    "--strict-markers",
    "--strict-config",
    "--disable-warnings",
    "--disable-pytest-warnings",
    "--runxfail",
    "--ff",
    "--failed-first",
    "--nf",
    "--new-first",
    "--cache-clear",
    "--full-trace",
    "--forked",
    "--cov-append",
    "--cov-branch",
    "--no-cov",
    "--no-cov-on-fail",
];

/// The long flags that leave the tests alone and take their value as the next word when it is not
/// joined with `=`.
const VALUE_FLAGS: &[&str] = &[
    "--numprocesses",
    "--dist",
    "--maxfail",
    "--durations",
    "--durations-min",
    "--tb",
    "--color",
    "--code-highlight",
    "--capture",
    "--import-mode",
    "--rootdir",
    "--basetemp",
    "--junitxml",
    "--junit-xml",
    "--log-level",
    "--log-cli-level",
    "--timeout",
    "--cov",
    "--cov-report",
    "--cov-config",
    "--cov-fail-under",
    "--confcutdir",
    "--override-ini",
    "--config-file",
    "--assert",
    "--show-capture",
];

/// The flags that report on the run or check its coverage, with or without a value: kept out of
/// the narrow command.
const REPORTING: &[&str] = &[
    "--durations",
    "--durations-min",
    "--junitxml",
    "--junit-xml",
    "--cov",
    "--cov-report",
    "--cov-config",
    "--cov-fail-under",
    "--cov-append",
    "--cov-branch",
    "--no-cov",
    "--no-cov-on-fail",
];

/// The letters of short flags that leave the tests alone and take no value: `-x` stops at the
/// first failure, `-v` and `-q` set the detail, `-s` turns capturing off, `-l` shows locals.
const SHORT_PLAIN_LETTERS: &str = "xvqsl";

/// The letters of short flags that leave the tests alone and take a value, the rest of the word or
/// the next word: a plugin (`-p`), a config file (`-c`), an ini option (`-o`), a warnings filter
/// (`-W`), the summary's report (`-r`) and the workers (`-n`).
const SHORT_VALUE_LETTERS: &str = "pcoWrn";

/// Whether the arguments after `pytest` select nothing; then they are the run's options, those that
/// report on it left out.
pub(super) fn whole(args: &[&str]) -> Option<Whole> {
    let mut options: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        at += 1;
        if !arg.starts_with('-') || arg == "-" || arg == "--" {
            return None;
        }
        let (name, joined) = arg
            .split_once('=')
            .map_or((arg, false), |(name, _)| (name, true));
        let takes_next = if name.starts_with("--") {
            let takes_value = VALUE_FLAGS.contains(&name);
            if !takes_value && (joined || !PLAIN_FLAGS.contains(&name)) {
                return None;
            }
            takes_value && !joined
        } else {
            short_flags(name)?.takes_next && !joined
        };
        let value = if takes_next {
            at += 1;
            Some(*args.get(at - 1)?)
        } else {
            None
        };
        if !REPORTING.contains(&name) {
            options.push(arg.to_owned());
            options.extend(value.map(str::to_owned));
        }
    }
    Some(Whole {
        options,
        ..Whole::default()
    })
}

/// What a bundle of short flags (`-xvs`, `-n4`, `-rA`) does.
struct Short {
    /// Whether the next word is the value of the last flag.
    takes_next: bool,
}

/// Reads the short flags of `name`; `None` when one of them is not a letter that leaves the tests
/// alone (`-k` and `-m` select, `-h` runs none).
fn short_flags(name: &str) -> Option<Short> {
    let letters = name.strip_prefix('-')?;
    for (at, letter) in letters.char_indices() {
        if SHORT_VALUE_LETTERS.contains(letter) {
            let after = at + letter.len_utf8();
            return Some(Short {
                takes_next: letters.get(after..).is_none_or(str::is_empty),
            });
        }
        if !SHORT_PLAIN_LETTERS.contains(letter) {
            return None;
        }
    }
    Some(Short { takes_next: false })
}

/// The Python files among `files`: pytest takes them as paths.
pub(super) fn narrow(_run: &SuiteRun, files: &[&str]) -> Option<Narrowed> {
    let mut found = Narrowed::default();
    for (place, file) in files.iter().enumerate() {
        let is_python = std::path::Path::new(file)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("py"));
        if is_python {
            found.add(place, (*file).to_owned());
        }
    }
    found.found()
}
