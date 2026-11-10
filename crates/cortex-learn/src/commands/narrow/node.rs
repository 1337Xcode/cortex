//! jest and vitest: which arguments select tests, and how a narrow command names test files. A
//! file filter, a name pattern (`-t`), `--changed`, `--shard` or a project selects; only a list of
//! flags known to leave the tests alone keeps a run whole, and any other flag makes it uncertain.
//! The listed flags are repeated in the narrow command, except coverage and reporters: a narrow
//! run's coverage falls short of thresholds the whole suite meets. A filter is a path to jest and a substring of
//! one to vitest, so a file name holding regular-expression characters is not given to jest.

use super::{Narrowed, SuiteRun, Whole};

/// The extensions of the files jest and vitest run.
const SCRIPT_EXTENSIONS: &[&str] = &["js", "jsx", "mjs", "cjs", "ts", "tsx", "mts", "cts"];

/// The jest flags that leave the tests alone and take no value.
const JEST_PLAIN: &[&str] = &[
    "--ci",
    "--coverage",
    "--silent",
    "--verbose",
    "--colors",
    "--no-colors",
    "--runInBand",
    "-i",
    "--forceExit",
    "--detectOpenHandles",
    "--no-cache",
    "--passWithNoTests",
    "--no-watchman",
];

/// The jest flags that take a value, `=` or the next word.
const JEST_VALUE: &[&str] = &["--maxWorkers", "-w", "--testTimeout"];

/// The vitest flags that leave the tests alone and take no value.
const VITEST_PLAIN: &[&str] = &[
    "--run",
    "--coverage",
    "--silent",
    "--no-color",
    "--passWithNoTests",
    "--globals",
];

/// The vitest flags that take a value, `=` or the next word.
const VITEST_VALUE: &[&str] = &["--reporter", "--pool", "--maxWorkers", "--bail"];

/// The flags that report on the run or measure its coverage, kept out of the narrow command.
const REPORTING: &[&str] = &["--coverage", "--reporter"];

/// The options of `args` when they hold only the flags of `plain` and `with_value`, from the first
/// up, those that report left out; `None` for any other word.
fn only_flags(args: &[&str], plain: &[&str], with_value: &[&str]) -> Option<Vec<String>> {
    let mut options: Vec<String> = Vec::new();
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        at += 1;
        let (name, joined) = arg
            .split_once('=')
            .map_or((arg, false), |(name, _)| (name, true));
        let value = if with_value.contains(&name) && !joined {
            at += 1;
            Some(*args.get(at - 1)?)
        } else if with_value.contains(&name) || plain.contains(&name) {
            None
        } else {
            return None;
        };
        if !REPORTING.contains(&name) {
            options.push(arg.to_owned());
            options.extend(value.map(str::to_owned));
        }
    }
    Some(options)
}

/// Whether the arguments after `jest` select nothing; then all of them are the run's options.
pub(super) fn whole_jest(args: &[&str]) -> Option<Whole> {
    only_flags(args, JEST_PLAIN, JEST_VALUE).map(|options| Whole {
        options,
        ..Whole::default()
    })
}

/// Whether the arguments after `vitest` select nothing: the `run` command or none, which in
/// watch mode is not a one-off run, so the narrow command always says `run`; then the flags are
/// the run's options.
pub(super) fn whole_vitest(args: &[&str]) -> Option<Whole> {
    let flags = match args.split_first() {
        Some((&"run", rest)) => rest,
        _ => args,
    };
    only_flags(flags, VITEST_PLAIN, VITEST_VALUE).map(|options| Whole {
        tail: vec!["run".to_owned()],
        options,
        ..Whole::default()
    })
}

/// Whether `file` is a script file jest and vitest run.
fn is_script(file: &str) -> bool {
    file.rsplit_once('.')
        .is_some_and(|(_, extension)| SCRIPT_EXTENSIONS.contains(&extension))
}

/// Whether `file` can be given to jest as a pattern that matches only itself and files that end
/// the same way: letters, digits and `_./-`.
fn is_pattern_safe(file: &str) -> bool {
    file.bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'/' | b'-'))
}

/// The script files among `files` that jest takes as patterns.
pub(super) fn narrow_jest(_run: &SuiteRun, files: &[&str]) -> Option<Narrowed> {
    scripts(files, is_pattern_safe)
}

/// The script files among `files`, which vitest takes as substrings of paths.
pub(super) fn narrow_vitest(_run: &SuiteRun, files: &[&str]) -> Option<Narrowed> {
    scripts(files, |_| true)
}

/// The script files among `files` that `is_taken` lets through, each its own argument.
fn scripts(files: &[&str], is_taken: fn(&str) -> bool) -> Option<Narrowed> {
    let mut found = Narrowed::default();
    for (place, file) in files.iter().enumerate() {
        if is_script(file) && is_taken(file) {
            found.add(place, (*file).to_owned());
        }
    }
    found.found()
}
