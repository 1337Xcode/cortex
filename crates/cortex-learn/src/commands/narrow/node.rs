//! jest and vitest: which arguments select tests, and how a narrow command names test files. A
//! file filter, a name pattern (`-t`), `--changed`, `--shard` or a project selects; only a list of
//! flags known to leave the tests alone keeps a run whole, and any other flag makes it uncertain,
//! as the narrow command drops what is not listed. A filter is a path to jest and a substring of
//! one to vitest, so a file name holding regular-expression characters is not given to jest.

use super::{SuiteRun, Whole};

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

/// Whether `args` hold only the flags of `plain` and `with_value`, from the first up.
fn only_flags(args: &[&str], plain: &[&str], with_value: &[&str]) -> bool {
    let mut at = 0;
    while let Some(&arg) = args.get(at) {
        at += 1;
        let (name, joined) = arg
            .split_once('=')
            .map_or((arg, false), |(name, _)| (name, true));
        if with_value.contains(&name) {
            at += usize::from(!joined);
        } else if !plain.contains(&name) {
            return false;
        }
    }
    true
}

/// Whether the arguments after `jest` select nothing.
pub(super) fn whole_jest(args: &[&str]) -> Option<Whole> {
    only_flags(args, JEST_PLAIN, JEST_VALUE).then(Whole::default)
}

/// Whether the arguments after `vitest` select nothing: the `run` command or none, which in
/// watch mode is not a one-off run, so the narrow command always says `run`.
pub(super) fn whole_vitest(args: &[&str]) -> Option<Whole> {
    let flags = match args.split_first() {
        Some((&"run", rest)) => rest,
        _ => args,
    };
    only_flags(flags, VITEST_PLAIN, VITEST_VALUE).then(|| Whole {
        tail: vec!["run".to_owned()],
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
pub(super) fn narrow_jest(_run: &SuiteRun, files: &[&str]) -> Option<Vec<String>> {
    let found: Vec<String> = files
        .iter()
        .filter(|file| is_script(file) && is_pattern_safe(file))
        .map(|file| (*file).to_owned())
        .collect();
    (!found.is_empty()).then_some(found)
}

/// The script files among `files`, which vitest takes as substrings of paths.
pub(super) fn narrow_vitest(_run: &SuiteRun, files: &[&str]) -> Option<Vec<String>> {
    let found: Vec<String> = files
        .iter()
        .filter(|file| is_script(file))
        .map(|file| (*file).to_owned())
        .collect();
    (!found.is_empty()).then_some(found)
}
